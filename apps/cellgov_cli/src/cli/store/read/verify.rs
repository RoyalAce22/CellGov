//! `firmware verify` and `title verify` -- re-hash an installed tree
//! against the record that describes it.
//!
//! A run that finds a divergence exits [`exit_codes::DIVERGED`] and
//! names every artefact that did not match, one per line.

use std::path::Path;

use cellgov_install::firmware_verify::{verify_firmware_entry, ModuleDivergence, ModuleFault};
use cellgov_install::keys::KeyVault;
use cellgov_install::store::{
    verify_record_tree, Artifact, Divergence, DivergenceKind, InstallRecord, KernelAbsence,
    StoreLayout, TitleId, VersionKey,
};

use crate::cli::exit::{CommandError, CommandExitCode};
use crate::cli::exit_codes;
use crate::cli::parse::OutputFormat;
use cellgov_boot::manifest::BASE_GAME_VER;
use cellgov_install::store::inventory::FirmwareEntry;

use super::collect::StoreView;
use super::model::{DivergenceDoc, VerifiedEntryDoc, VerifyDoc, KERNEL_NOT_RECORDED};
use super::{emit, view};

/// `cellgov firmware verify <VERSION>`
pub(crate) fn firmware_verify(
    root: &Path,
    version: &str,
    format: OutputFormat,
) -> Result<CommandExitCode, CommandError> {
    let view = view(root)?;
    let entry = view.inventory.firmware(version).ok_or_else(|| {
        CommandError::failed(format!(
            "no firmware {version:?} is installed; installed: {}",
            super::key_list(&view.inventory.firmware_versions())
        ))
    })?;
    let keys = cellgov_install::keys::KeyVault::load_for_vfs(root)
        .map_err(|error| CommandError::failed(error.to_string()))?;
    let entry = firmware_entry_doc(&view, entry, &keys)?;

    let doc = VerifyDoc {
        format_version: view.format_version(),
        store: view.store_label(),
        subject: version.to_string(),
        clean: entry.divergences.is_empty(),
        entries: vec![entry],
    };
    finish(&doc, format, &format!("firmware {version}"))
}

/// Verify one installed firmware entry with the existing module and stored-kernel rules.
pub(super) fn firmware_entry_doc(
    view: &StoreView,
    entry: &FirmwareEntry,
    keys: &KeyVault,
) -> Result<VerifiedEntryDoc, CommandError> {
    let version = &entry.version;
    // An empty manifest refuses as `EmptyManifest`, so an entry that
    // verifies has examined at least one module.
    let checked = verify_firmware_entry(&entry.entry_dir, entry.core_os.as_ref(), keys)
        .map_err(|error| CommandError::failed(format!("firmware verify {version}: {error}")))?;
    Ok(VerifiedEntryDoc {
        entry: version.to_string(),
        matched: checked.report.matched,
        divergences: checked
            .report
            .divergences
            .iter()
            .map(|f| module_fault_doc(view, f))
            .collect(),
        kernel_omission: checked.kernel_absence.map(|absence| match absence {
            KernelAbsence::NotRecorded => KERNEL_NOT_RECORDED.to_string(),
            KernelAbsence::Omitted(reason) => reason.unwrap_or("not unpacked").to_string(),
        }),
    })
}

/// `cellgov title verify <TITLE_ID> [--ver V]`
pub(crate) fn title_verify(
    root: &Path,
    title_id: &str,
    ver: Option<&str>,
    format: OutputFormat,
) -> Result<CommandExitCode, CommandError> {
    let view = view(root)?;
    let entry = view.inventory.title(title_id).ok_or_else(|| {
        CommandError::failed(format!(
            "no title {title_id:?} is installed; installed: {}",
            super::key_list(
                &view
                    .inventory
                    .titles()
                    .map(|t| t.title_id.clone())
                    .collect::<Vec<_>>()
            )
        ))
    })?;
    let key = TitleId::new(title_id)
        .map_err(|error| CommandError::failed(format!("title verify {title_id}: {error}")))?;

    // Which entries the run covers: everything installed, or the one
    // version `--ver` names.
    let mut wanted: Vec<(String, Artifact)> = Vec::new();
    match ver {
        None => {
            if entry.base.is_some() {
                wanted.push((
                    BASE_GAME_VER.to_string(),
                    Artifact::TitleBase {
                        title_id: key.clone(),
                    },
                ));
            }
            for version in entry.updates.keys() {
                wanted.push((version.clone(), update_artifact(&key, version)?));
            }
        }
        Some(BASE_GAME_VER) => {
            if entry.base.is_none() {
                return Err(CommandError::failed(format!(
                    "title {title_id} has no base installed; installed: {}",
                    super::key_list(&entry.candidates())
                )));
            }
            wanted.push((
                BASE_GAME_VER.to_string(),
                Artifact::TitleBase {
                    title_id: key.clone(),
                },
            ));
        }
        Some(version) => {
            if !entry.updates.contains_key(version) {
                return Err(CommandError::failed(format!(
                    "title {title_id} has no version {version:?} installed; installed: {}",
                    super::key_list(&entry.candidates())
                )));
            }
            wanted.push((version.to_string(), update_artifact(&key, version)?));
        }
    }
    if wanted.is_empty() {
        return Err(CommandError::failed(format!(
            "title {title_id} has a records directory but no installed entry to verify"
        )));
    }

    let layout = StoreLayout::new(root);
    let live_rap_dir = layout.live_exdata_dir();
    let mut entries = Vec::with_capacity(wanted.len());
    for (label, artifact) in &wanted {
        let record_path = layout.record_path(artifact);
        let record = load_record(&record_path)?;
        let tree_dir = layout.resolve_store_path(&record.artifact.store_path);
        let rap = record.rap.as_ref().map(|r| live_rap_dir.join(&r.filename));
        let report = verify_record_tree(&tree_dir, rap.as_deref(), &record)
            .map_err(|error| CommandError::failed(format!("title verify {title_id}: {error}")))?;
        entries.push(VerifiedEntryDoc {
            entry: label.clone(),
            matched: report.matched,
            divergences: report
                .divergences
                .iter()
                .map(|d| divergence_doc(&view, d))
                .collect(),
            kernel_omission: None,
        });
    }

    let doc = VerifyDoc {
        format_version: view.format_version(),
        store: view.store_label(),
        subject: title_id.to_string(),
        clean: entries.iter().all(|e| e.divergences.is_empty()),
        entries,
    };
    finish(&doc, format, &format!("title {title_id}"))
}

fn update_artifact(title_id: &TitleId, version: &str) -> Result<Artifact, CommandError> {
    Ok(Artifact::TitleUpdate {
        title_id: title_id.clone(),
        // The version came from a record filed under this key, so the
        // store already accepted it as a directory name.
        version: VersionKey::new(version).map_err(|error| {
            CommandError::failed(format!("installed update version {version:?}: {error}"))
        })?,
    })
}

fn load_record(path: &Path) -> Result<InstallRecord, CommandError> {
    let text = std::fs::read_to_string(path).map_err(|error| {
        CommandError::failed(format!("read install record {}: {error}", path.display()))
    })?;
    InstallRecord::parse(&text).map_err(|error| {
        CommandError::failed(format!("install record {}: {error}", path.display()))
    })
}

fn divergence_doc(view: &StoreView, d: &Divergence) -> DivergenceDoc {
    match &d.kind {
        DivergenceKind::Missing => DivergenceDoc {
            path: view.rel(&d.path),
            kind: "missing".to_string(),
            expected: None,
            found: None,
            reason: None,
        },
        DivergenceKind::Modified { expected, found } => DivergenceDoc {
            path: view.rel(&d.path),
            kind: "modified".to_string(),
            expected: Some(expected.to_hex()),
            found: Some(found.to_hex()),
            reason: None,
        },
    }
}

fn module_fault_doc(view: &StoreView, f: &ModuleFault) -> DivergenceDoc {
    match &f.kind {
        ModuleDivergence::Missing => DivergenceDoc {
            path: view.rel(&f.path),
            kind: "missing".to_string(),
            expected: None,
            found: None,
            reason: None,
        },
        ModuleDivergence::Modified { expected, found } => DivergenceDoc {
            path: view.rel(&f.path),
            kind: "modified".to_string(),
            expected: Some(expected.to_hex()),
            found: Some(found.to_hex()),
            reason: None,
        },
        ModuleDivergence::NoImage { reason } => DivergenceDoc {
            path: view.rel(&f.path),
            kind: "no-image".to_string(),
            expected: None,
            found: None,
            reason: Some(reason.clone()),
        },
    }
}

/// A pass that examined no artefact at all.
///
/// `clean` is the absence of divergence, so a record naming no file
/// makes it vacuously true.
fn checked_nothing(doc: &VerifyDoc) -> bool {
    doc.matched() + doc.diverged() == 0
}

fn finish(
    doc: &VerifyDoc,
    format: OutputFormat,
    subject: &str,
) -> Result<CommandExitCode, CommandError> {
    if checked_nothing(doc) {
        return Err(CommandError::failed(format!(
            "{subject}: the install record names no file, so the pass examined nothing; \
             reinstall the entry to write a record that covers its tree"
        )));
    }
    emit(format, doc, || print!("{}", render(doc, subject)))?;
    Ok(CommandExitCode::new(if doc.clean {
        0
    } else {
        exit_codes::DIVERGED
    }))
}

fn render(doc: &VerifyDoc, subject: &str) -> String {
    let mut out = String::new();
    for entry in &doc.entries {
        out.push_str(&render_entry(entry));
    }
    let checked = doc.matched() + doc.diverged();
    out.push_str(&if doc.clean {
        format!("{subject}: {checked} artefact(s) match their record\n")
    } else {
        format!(
            "{subject}: {} of {checked} artefact(s) diverged\n",
            doc.diverged()
        )
    });
    out
}

/// Render one installed entry with the shared verification vocabulary.
pub(super) fn render_entry(entry: &VerifiedEntryDoc) -> String {
    let mut out = String::new();
    // An omission is no divergence: the pass found nothing wrong.
    // The line keeps a clean summary from claiming a kernel the pass
    // did not check.
    if let Some(why) = &entry.kernel_omission {
        out.push_str(&format!("{}: kernel not checked: {why}\n", entry.entry));
    }
    for divergence in &entry.divergences {
        out.push_str(
            &match (&divergence.expected, &divergence.found, &divergence.reason) {
                (Some(expected), Some(found), _) => format!(
                    "{}: modified (recorded {expected}, found {found})\n",
                    divergence.path
                ),
                (_, _, Some(reason)) => {
                    format!("{}: no module image ({reason})\n", divergence.path)
                }
                _ => format!("{}: missing\n", divergence.path),
            },
        );
    }
    out
}

#[cfg(test)]
#[path = "tests/verify_tests.rs"]
mod tests;
