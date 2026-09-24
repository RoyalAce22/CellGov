//! Verifies acquired PUP files and installed firmware against the LV2 archive.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use cellgov_install::manifest::Sha256;
use cellgov_lv2::archive::{self, PupRow, PUP};

use crate::cli::exit::{CommandError, CommandExitCode};
use crate::cli::exit_codes;
use crate::cli::parse::OutputFormat;

use super::model::{
    store_rel, PupEntryDoc, PupMismatchDoc, PupVerifyDoc, VerifiedEntryDoc, STORE_FORMAT_VERSION,
};
use super::render::emit;
use super::verify::{firmware_entry_doc, render_entry};
use super::view;

const PUP_TSV: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../docs/lv2/tables/pup.tsv"
));

struct ScannedPup {
    path: String,
    sha256: String,
    size_bytes: u64,
    fw: Result<(String, String), String>,
}

fn archive_rows() -> Result<Vec<PupRow>, CommandError> {
    let table = archive::parse(&PUP, PUP_TSV).map_err(|error| {
        CommandError::failed(format!(
            "compiled docs/lv2/tables/pup.tsv is invalid: {error}"
        ))
    })?;
    let rows = archive::pup_rows(&table);
    archive::check_pup_rows(&rows).map_err(|error| {
        CommandError::failed(format!(
            "compiled docs/lv2/tables/pup.tsv is invalid: {error}"
        ))
    })?;
    Ok(rows)
}

fn pup_paths(dir: &Path) -> Result<Vec<PathBuf>, CommandError> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) -> Result<(), CommandError> {
        let entries = std::fs::read_dir(dir).map_err(|error| {
            CommandError::failed(format!("read PUP directory {}: {error}", dir.display()))
        })?;
        for entry in entries {
            let entry = entry.map_err(|error| {
                CommandError::failed(format!("read PUP directory {}: {error}", dir.display()))
            })?;
            let path = entry.path();
            let kind = entry.file_type().map_err(|error| {
                CommandError::failed(format!("inspect PUP path {}: {error}", path.display()))
            })?;
            if kind.is_dir() {
                walk(&path, out)?;
            } else if kind.is_file()
                && path.extension().is_some_and(|extension| {
                    extension.to_string_lossy().eq_ignore_ascii_case("pup")
                })
            {
                out.push(path);
            }
        }
        Ok(())
    }

    if !dir.is_dir() {
        return Err(CommandError::failed(format!(
            "firmware PUP directory {} is not a directory",
            dir.display()
        )));
    }
    let mut out = Vec::new();
    walk(dir, &mut out)?;
    out.sort();
    Ok(out)
}

fn scan_pup(pup_directory: &Path, path: &Path) -> Result<ScannedPup, CommandError> {
    let bytes = filebuffer::FileBuffer::open(path)
        .map_err(|error| CommandError::failed(format!("read PUP {}: {error}", path.display())))?;
    let sha256 = Sha256(cellgov_install::manifest::sha256_of(&bytes)).to_hex();
    let fw = cellgov_install::pup::parse(&bytes)
        .and_then(|pup| {
            let version = cellgov_install::pup::version_key(&bytes, &pup)?;
            Ok((version, format!("0x{:016x}", pup.image_version)))
        })
        .map_err(|error| error.to_string());
    Ok(ScannedPup {
        path: store_rel(pup_directory, path),
        sha256,
        size_bytes: bytes.len() as u64,
        fw,
    })
}

fn expected_doc(row: &PupRow, path: Option<String>) -> PupEntryDoc {
    PupEntryDoc {
        fw: row.fw.clone(),
        pup_sha256: row.pup_sha256.clone(),
        size_bytes: row.size_bytes,
        image_version: row.image_version.clone(),
        path,
    }
}

fn classify(
    rows: &[PupRow],
    scanned: &[ScannedPup],
) -> (Vec<PupEntryDoc>, Vec<PupEntryDoc>, Vec<PupMismatchDoc>) {
    let by_hash: BTreeMap<&str, &PupRow> = rows
        .iter()
        .map(|row| (row.pup_sha256.as_str(), row))
        .collect();
    let mut by_fw: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for row in rows {
        by_fw
            .entry(row.fw.as_str())
            .or_default()
            .push(row.pup_sha256.as_str());
    }
    let mut seen = BTreeSet::new();
    let mut present = Vec::new();
    let mut mismatched = Vec::new();
    for found in scanned {
        if let Some(row) = by_hash.get(found.sha256.as_str()) {
            let first = seen.insert(row.pup_sha256.as_str());
            match &found.fw {
                Ok((fw, image_version))
                    if fw == &row.fw
                        && image_version == &row.image_version
                        && found.size_bytes == row.size_bytes =>
                {
                    if first {
                        present.push(expected_doc(row, Some(found.path.clone())));
                    }
                }
                Ok((fw, image_version)) => mismatched.push(PupMismatchDoc {
                    subject: found.path.clone(),
                    kind: "metadata".to_string(),
                    fw: Some(fw.clone()),
                    expected: vec![format!(
                        "fw {}, size {}, image {}",
                        row.fw, row.size_bytes, row.image_version
                    )],
                    found: Some(format!(
                        "fw {fw}, size {}, image {image_version}",
                        found.size_bytes
                    )),
                    reason: None,
                }),
                Err(reason) => mismatched.push(PupMismatchDoc {
                    subject: found.path.clone(),
                    kind: "invalid-pup".to_string(),
                    fw: None,
                    expected: vec![row.pup_sha256.clone()],
                    found: Some(found.sha256.clone()),
                    reason: Some(reason.clone()),
                }),
            }
            continue;
        }
        match &found.fw {
            Ok((fw, _)) => mismatched.push(PupMismatchDoc {
                subject: found.path.clone(),
                kind: "sha256".to_string(),
                fw: Some(fw.clone()),
                expected: by_fw.get(fw.as_str()).map_or_else(Vec::new, |hashes| {
                    hashes.iter().map(|hash| (*hash).to_string()).collect()
                }),
                found: Some(found.sha256.clone()),
                reason: None,
            }),
            Err(reason) => mismatched.push(PupMismatchDoc {
                subject: found.path.clone(),
                kind: "invalid-pup".to_string(),
                fw: None,
                expected: Vec::new(),
                found: Some(found.sha256.clone()),
                reason: Some(reason.clone()),
            }),
        }
    }
    let missing = rows
        .iter()
        .filter(|row| !seen.contains(row.pup_sha256.as_str()))
        .map(|row| expected_doc(row, None))
        .collect();
    present.sort_by(|a, b| a.pup_sha256.cmp(&b.pup_sha256));
    mismatched.sort_by(|a, b| a.subject.cmp(&b.subject));
    (present, missing, mismatched)
}

fn installed_identity_mismatches(
    installed_version: &str,
    record_hash: &str,
    manifest_hash: &str,
    manifest_image: &str,
    row: &PupRow,
) -> Vec<PupMismatchDoc> {
    let subject = format!("installed firmware {installed_version}");
    let mut mismatched = Vec::new();
    if installed_version != row.fw {
        mismatched.push(PupMismatchDoc {
            subject: subject.clone(),
            kind: "firmware-version".to_string(),
            fw: Some(installed_version.to_string()),
            expected: vec![row.fw.clone()],
            found: Some(installed_version.to_string()),
            reason: None,
        });
    }
    if manifest_image != row.image_version {
        mismatched.push(PupMismatchDoc {
            subject: subject.clone(),
            kind: "image-version".to_string(),
            fw: Some(installed_version.to_string()),
            expected: vec![row.image_version.clone()],
            found: Some(manifest_image.to_string()),
            reason: None,
        });
    }
    if record_hash != manifest_hash {
        mismatched.push(PupMismatchDoc {
            subject,
            kind: "source-sha256".to_string(),
            fw: Some(installed_version.to_string()),
            expected: vec![row.pup_sha256.clone()],
            found: Some(record_hash.to_string()),
            reason: None,
        });
    }
    mismatched
}

fn installed_manifest_version_mismatch(
    installed_version: &str,
    manifest_version: &str,
    row: &PupRow,
) -> Option<PupMismatchDoc> {
    if manifest_version == row.fw {
        return None;
    }
    // The boot identity gate rejects this same stale-manifest state in
    // `cellgov_boot::compose`.
    Some(PupMismatchDoc {
        subject: format!("installed firmware {installed_version}"),
        kind: "manifest-version".to_string(),
        fw: Some(manifest_version.to_string()),
        expected: vec![row.fw.clone()],
        found: Some(manifest_version.to_string()),
        reason: None,
    })
}

fn pup_set_is_clean(
    missing: &[PupEntryDoc],
    mismatched: &[PupMismatchDoc],
    installed: &[VerifiedEntryDoc],
) -> bool {
    missing.is_empty()
        && mismatched.is_empty()
        && installed.iter().all(|entry| entry.divergences.is_empty())
}

pub(crate) fn firmware_verify_pups(
    root: &Path,
    pup_directory: &Path,
    format: OutputFormat,
) -> Result<CommandExitCode, CommandError> {
    let rows = archive_rows()?;
    let scanned: Vec<ScannedPup> = pup_paths(pup_directory)?
        .into_iter()
        .map(|path| scan_pup(pup_directory, &path))
        .collect::<Result<_, _>>()?;
    let (present, missing, mut mismatched) = classify(&rows, &scanned);
    let by_hash: BTreeMap<&str, &PupRow> = rows
        .iter()
        .map(|row| (row.pup_sha256.as_str(), row))
        .collect();

    let store = view(root)?;
    let mut candidates = Vec::new();
    for entry in store.inventory.firmware_entries() {
        let manifest = cellgov_install::firmware_verify::load_manifest(&entry.dev_flash_dir())
            .map_err(|error| {
                CommandError::failed(format!("firmware verify {}: {error}", entry.version))
            })?;
        let manifest_hash = manifest.firmware.pup_sha256.to_hex();
        if let Some(row) = by_hash.get(manifest_hash.as_str()) {
            candidates.push((entry, *row, manifest, manifest_hash));
        }
    }
    let keys = if candidates.is_empty() {
        None
    } else {
        Some(
            cellgov_install::keys::KeyVault::load_for_vfs(root)
                .map_err(|error| CommandError::failed(error.to_string()))?,
        )
    };
    let mut installed = Vec::new();
    for (entry, row, manifest, manifest_hash) in candidates {
        if let Some(mismatch) =
            installed_manifest_version_mismatch(&entry.version, &manifest.firmware.version, row)
        {
            mismatched.push(mismatch);
        }
        mismatched.extend(installed_identity_mismatches(
            &entry.version,
            &entry.pup_sha256,
            &manifest_hash,
            &manifest.firmware.image_version,
            row,
        ));
        let Some(keys) = keys.as_ref() else {
            return Err(CommandError::failed(
                "firmware verify-pups: installed candidates exist but the key vault was not loaded",
            ));
        };
        installed.push(firmware_entry_doc(&store, entry, keys)?);
    }
    mismatched.sort_by(|a, b| a.subject.cmp(&b.subject));
    let clean = pup_set_is_clean(&missing, &mismatched, &installed);
    let doc = PupVerifyDoc {
        format_version: STORE_FORMAT_VERSION,
        pup_directory: pup_directory.display().to_string(),
        present,
        missing,
        mismatched,
        installed,
        clean,
    };
    emit(format, &doc, || print!("{}", render(&doc)))?;
    Ok(CommandExitCode::new(if doc.clean {
        0
    } else {
        exit_codes::DIVERGED
    }))
}

fn render(doc: &PupVerifyDoc) -> String {
    let mut out = String::new();
    out.push_str("present:\n");
    for row in &doc.present {
        out.push_str(&format!(
            "  fw {}  {}  {}\n",
            row.fw,
            row.pup_sha256,
            row.path.as_deref().unwrap_or("--")
        ));
    }
    out.push_str("missing:\n");
    for row in &doc.missing {
        out.push_str(&format!("  fw {}  {}\n", row.fw, row.pup_sha256));
    }
    out.push_str("mismatched:\n");
    for row in &doc.mismatched {
        let expected = if row.expected.is_empty() {
            "<no archive row>".to_string()
        } else {
            row.expected.join(" or ")
        };
        out.push_str(&format!(
            "  {}: {} (expected {expected}, found {}){}\n",
            row.subject,
            row.kind,
            row.found.as_deref().unwrap_or("--"),
            row.reason
                .as_deref()
                .map_or_else(String::new, |reason| format!("; {reason}")),
        ));
    }
    if !doc.installed.is_empty() {
        out.push_str("installed:\n");
        for entry in &doc.installed {
            for line in render_entry(entry).lines() {
                out.push_str(&format!("  {line}\n"));
            }
        }
    }
    out.push_str(&format!(
        "installed firmware: {} present, {} missing, {} mismatched; {} installed entr{} checked\n",
        doc.present.len(),
        doc.missing.len(),
        doc.mismatched.len(),
        doc.installed.len(),
        if doc.installed.len() == 1 { "y" } else { "ies" },
    ));
    out
}

#[cfg(test)]
#[path = "tests/pup_tests.rs"]
mod tests;
