//! `firmware kernels` -- decrypt every stored LV2 kernel and report the
//! vault's coverage of the store, version by version.
//!
//! A version whose kernel the vault cannot open is the normal case of
//! a store that spans key eras, so the run reports it and continues.
//! The table distinguishes a missing key from a failed decrypt and
//! from an entry that never unpacked its kernel.

#[cfg(feature = "decrypt")]
use std::collections::BTreeMap;
use std::path::Path;
#[cfg(feature = "decrypt")]
use std::path::PathBuf;

use crate::cli::exit::{CommandError, CommandExitCode};
use crate::cli::parse::OutputFormat;

#[cfg(feature = "decrypt")]
use super::model::{KernelCoverageDoc, KernelCoverageEntryDoc, KERNEL_NOT_RECORDED_REASON};
#[cfg(feature = "decrypt")]
use super::{emit, view};

/// Exit status when a stored kernel yielded no ELF for a reason other
/// than a missing key.
#[cfg(feature = "decrypt")]
const EXIT_KERNEL_NOT_DECRYPTED: i32 = crate::cli::exit_codes::command_specific(41);

/// The state of an entry that stored no kernel; the other four states
/// come from `KernelCoverage::label`.
#[cfg(feature = "decrypt")]
const NOT_UNPACKED: &str = "not_unpacked";

#[cfg(feature = "decrypt")]
const NOT_INSTALLED: &str = "not_installed";

/// The states a row takes, in the order the summary line lists them.
#[cfg(feature = "decrypt")]
const STATES: [&str; 6] = [
    "decrypted",
    "no_key",
    NOT_INSTALLED,
    NOT_UNPACKED,
    "unreadable",
    "failed",
];

/// The states that make the run exit [`EXIT_KERNEL_NOT_DECRYPTED`].
#[cfg(feature = "decrypt")]
const NOT_DECRYPTED_STATES: [&str; 2] = ["unreadable", "failed"];

#[cfg(feature = "decrypt")]
const REPORT_REL: &str = ".cellgov/firmware-kernel-coverage.json";

#[cfg(feature = "decrypt")]
const FIRMWARE_TSV: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../docs/lv2/tables/firmware.tsv"
));

#[cfg(not(feature = "decrypt"))]
pub(crate) fn firmware_kernels(
    _root: &Path,
    _format: OutputFormat,
) -> Result<CommandExitCode, CommandError> {
    Err(CommandError::failed(
        crate::cli::store::StoreCliError::DecryptFeatureDisabled {
            command: "firmware kernels".to_string(),
        }
        .to_string(),
    ))
}

/// `cellgov firmware kernels`
#[cfg(feature = "decrypt")]
pub(crate) fn firmware_kernels(
    root: &Path,
    format: OutputFormat,
) -> Result<CommandExitCode, CommandError> {
    use cellgov_install::kernel_decrypt::{entry_kernel_coverage, EntryKernelCoverage};
    use cellgov_install::keys::KeyVault;
    use cellgov_install::store::KernelAbsence;

    let view = view(root)?;
    let location = KeyVault::locate_from(std::env::var_os(crate::env_vars::KEYS), root)
        .map_err(|error| CommandError::failed(error.to_string()))?;
    let keys = KeyVault::load_from_path(&location)
        .map_err(|error| CommandError::failed(error.to_string()))?;

    let installed: BTreeMap<String, KernelCoverageEntryDoc> = view
        .inventory
        .firmware_entries()
        .map(|entry| {
            let mut doc = entry_doc(&entry.version, NOT_UNPACKED);
            match entry_kernel_coverage(&entry.entry_dir, entry.core_os.as_ref(), &keys) {
                EntryKernelCoverage::Attempted(coverage) => apply_coverage(&mut doc, coverage),
                // The state column already says "not unpacked".
                EntryKernelCoverage::Absent(KernelAbsence::NotRecorded) => {
                    doc.detail = Some(KERNEL_NOT_RECORDED_REASON.to_string());
                }
                EntryKernelCoverage::Absent(KernelAbsence::Omitted(reason)) => {
                    doc.detail = Some(reason.map_or_else(
                        || "the install stored no kernel and named no reason".to_string(),
                        str::to_string,
                    ));
                }
            }
            (entry.version.clone(), doc)
        })
        .collect();
    let entries = coverage_docs(&archive_versions()?, installed);

    let doc = KernelCoverageDoc {
        format_version: view.format_version(),
        store: view.store_label(),
        vault: location.display().to_string(),
        entries,
    };
    let report =
        write_report(root, &doc).map_err(|error| CommandError::failed(error.to_string()))?;
    emit(format, &doc, || print!("{}", render(&doc, &report)))?;
    Ok(CommandExitCode::new(exit_status(&doc)))
}

#[cfg(feature = "decrypt")]
fn archive_versions() -> Result<Vec<String>, CommandError> {
    use cellgov_lv2_archive::{self as archive, FIRMWARE};

    let table = archive::parse(&FIRMWARE, FIRMWARE_TSV)
        .map_err(|error| CommandError::failed(format!("compiled firmware.tsv: {error}")))?;
    let rows = archive::firmware_rows(&table);
    archive::check_firmware_rows(&rows)
        .map_err(|error| CommandError::failed(format!("compiled firmware.tsv: {error}")))?;
    Ok(rows.into_iter().map(|row| row.fw).collect())
}

/// The report's rows: every archive version, filled with its installed
/// entry or marked not installed, then the installs the archive does
/// not name.
#[cfg(feature = "decrypt")]
fn coverage_docs(
    archive_versions: &[String],
    installed: BTreeMap<String, KernelCoverageEntryDoc>,
) -> Vec<KernelCoverageEntryDoc> {
    cellgov_install::kernel_decrypt::coverage_rows(archive_versions, installed)
        .into_iter()
        .map(|(version, doc)| doc.unwrap_or_else(|| entry_doc(&version, NOT_INSTALLED)))
        .collect()
}

/// A row naming `version` in `state`, with every other field empty.
#[cfg(feature = "decrypt")]
fn entry_doc(version: &str, state: &str) -> KernelCoverageEntryDoc {
    KernelCoverageEntryDoc {
        version: version.to_string(),
        state: state.to_string(),
        detail: None,
        kernel_version: None,
        elf_bytes: None,
        elf_sha256: None,
    }
}

#[cfg(feature = "decrypt")]
fn report_path(root: &Path) -> PathBuf {
    REPORT_REL
        .split('/')
        .fold(root.to_path_buf(), |path, part| path.join(part))
}

#[cfg(feature = "decrypt")]
fn write_report(
    root: &Path,
    doc: &KernelCoverageDoc,
) -> Result<PathBuf, crate::cli::store::StoreCliError> {
    use crate::cli::store::StoreCliError;

    let path = report_path(root);
    let dir = root.join(".cellgov");
    std::fs::create_dir_all(&dir)
        .map_err(|source| StoreCliError::KernelCoverageDirCreateFailed { path: dir, source })?;
    let mut json = serde_json::to_vec_pretty(doc)
        .map_err(|source| StoreCliError::KernelCoverageSerializeFailed { source })?;
    json.push(b'\n');
    std::fs::write(&path, json).map_err(|source| StoreCliError::KernelCoverageWriteFailed {
        path: path.clone(),
        source,
    })?;
    Ok(path)
}

#[cfg(feature = "decrypt")]
fn apply_coverage(
    doc: &mut KernelCoverageEntryDoc,
    coverage: cellgov_install::kernel_decrypt::KernelCoverage,
) {
    use cellgov_install::kernel_decrypt::KernelCoverage;
    use cellgov_install::keys::version_label;

    doc.state = coverage.label().to_string();
    match coverage {
        KernelCoverage::Decrypted {
            version,
            elf_len,
            elf_sha256,
        } => {
            doc.kernel_version = Some(version_label(version));
            doc.elf_bytes = Some(elf_len);
            doc.elf_sha256 = Some(elf_sha256.to_hex());
        }
        KernelCoverage::NoKey { version, missing } => {
            doc.kernel_version = version.map(version_label);
            doc.detail = Some(missing);
        }
        KernelCoverage::Unreadable { reason } => doc.detail = Some(reason),
        KernelCoverage::Failed { version, reason } => {
            doc.kernel_version = version.map(version_label);
            doc.detail = Some(reason);
        }
    }
}

/// 0 unless a row is in one of [`NOT_DECRYPTED_STATES`].
#[cfg(feature = "decrypt")]
fn exit_status(doc: &KernelCoverageDoc) -> i32 {
    if NOT_DECRYPTED_STATES.iter().any(|s| doc.count(s) > 0) {
        EXIT_KERNEL_NOT_DECRYPTED
    } else {
        0
    }
}

#[cfg(feature = "decrypt")]
fn render(doc: &KernelCoverageDoc, report: &Path) -> String {
    let mut out = format!(
        "coverage report: {}\nkey vault: {}\n  VERSION  KERNEL        DETAIL\n",
        report.display(),
        doc.vault
    );
    for entry in &doc.entries {
        let detail = match (&entry.elf_bytes, &entry.elf_sha256, &entry.detail) {
            (Some(bytes), Some(sha256), _) => format!(
                "{} ELF (header names firmware {}), sha256 {sha256}",
                super::human_bytes(*bytes as u64),
                entry.kernel_version.as_deref().unwrap_or("?"),
            ),
            (_, _, Some(detail)) => detail.clone(),
            _ => String::new(),
        };
        out.push_str(&format!(
            "  {:<7}  {:<12}  {detail}\n",
            entry.version,
            entry.state.replace('_', " ")
        ));
    }
    let tally: Vec<String> = STATES
        .iter()
        .map(|state| format!("{} {}", doc.count(state), state.replace('_', " ")))
        .collect();
    out.push_str(&format!("states: {}\n", tally.join(", ")));
    out
}

#[cfg(all(test, feature = "decrypt"))]
#[path = "tests/kernels_tests.rs"]
mod tests;
