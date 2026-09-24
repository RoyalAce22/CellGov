//! Verifies acquired PUP files and installed firmware against the LV2 archive.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use cellgov_install::pup_verify::{
    classify, installed_mismatches, ArchivePup, InstalledClaims, PupMismatch, ScannedPup,
};

use crate::cli::exit::{CommandError, CommandExitCode};
use crate::cli::exit_codes;
use crate::cli::parse::OutputFormat;

use super::model::{
    store_rel, PupEntryDoc, PupMismatchDoc, PupVerifyDoc, VerifiedEntryDoc, STORE_FORMAT_VERSION,
};
use super::render::emit;
use super::verify::{firmware_entry_doc, render_entry};
use super::view;

/// The compiled archive's PUP rows, as the verifier reads them.
fn archive_rows() -> Result<Vec<ArchivePup>, CommandError> {
    let rows = crate::lv2_tables::committed_pup_rows()
        .map_err(|error| CommandError::failed(error.to_string()))?;
    Ok(rows
        .into_iter()
        .map(|row| ArchivePup {
            pup_sha256: row.pup_sha256,
            fw: row.fw,
            size_bytes: row.size_bytes,
            image_version: row.image_version,
        })
        .collect())
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
    Ok(ScannedPup::of(store_rel(pup_directory, path), &bytes))
}

fn expected_doc(row: &ArchivePup, path: Option<String>) -> PupEntryDoc {
    PupEntryDoc {
        fw: row.fw.clone(),
        pup_sha256: row.pup_sha256.clone(),
        size_bytes: row.size_bytes,
        image_version: row.image_version.clone(),
        path,
    }
}

fn mismatch_doc(mismatch: PupMismatch) -> PupMismatchDoc {
    PupMismatchDoc {
        subject: mismatch.subject,
        kind: mismatch.kind.label().to_string(),
        fw: mismatch.fw,
        expected: mismatch.expected,
        found: mismatch.found,
        reason: mismatch.reason,
    }
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
    let sorted = classify(&rows, &scanned);
    let present = sorted
        .present
        .into_iter()
        .map(|(row, path)| expected_doc(row, Some(path)))
        .collect();
    let missing: Vec<PupEntryDoc> = sorted
        .missing
        .into_iter()
        .map(|row| expected_doc(row, None))
        .collect();
    let mut mismatched: Vec<PupMismatchDoc> =
        sorted.mismatched.into_iter().map(mismatch_doc).collect();
    let by_hash: BTreeMap<&str, &ArchivePup> = rows
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
        mismatched.extend(
            installed_mismatches(
                &InstalledClaims {
                    version: &entry.version,
                    record_sha256: &entry.pup_sha256,
                    manifest_version: &manifest.firmware.version,
                    manifest_sha256: &manifest_hash,
                    manifest_image_version: &manifest.firmware.image_version,
                },
                row,
            )
            .into_iter()
            .map(mismatch_doc),
        );
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
