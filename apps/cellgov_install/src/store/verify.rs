//! Re-hash an installed tree against the record that describes it.
//!
//! The record's `[files]` keys are relative to the entry directory its
//! `store_path` names, so one walk covers a base tree and an update
//! tree alike. The walk checks the RAP with them when the record
//! carries one and the live license directory still holds it.

use std::path::{Path, PathBuf};

use crate::game_install::sha256_of;
use crate::manifest::Sha256 as HexSha256;
use crate::store::record::InstallRecord;

/// Why one recorded artefact did not match the live tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DivergenceKind {
    /// The record lists the file; the tree does not hold it.
    Missing,
    /// The file is there under a different hash.
    Modified {
        /// Hash the record holds.
        expected: HexSha256,
        /// Hash found on disk.
        found: HexSha256,
    },
}

/// One recorded artefact that did not match.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Divergence {
    /// The path the check examined.
    pub path: PathBuf,
    /// How it diverged.
    pub kind: DivergenceKind,
}

impl std::fmt::Display for Divergence {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.kind {
            DivergenceKind::Missing => write!(f, "{}: missing", self.path.display()),
            DivergenceKind::Modified { expected, found } => write!(
                f,
                "{}: modified (recorded {}, found {})",
                self.path.display(),
                expected.to_hex(),
                found.to_hex()
            ),
        }
    }
}

/// What a verification pass examined.
///
/// `matched + divergences.len()` is every file the record lists, plus
/// the RAP when the record names one and the live license directory
/// still holds it.
#[derive(Debug, Clone, Default)]
pub struct VerifyReport {
    /// Recorded artefacts whose hash matched.
    pub matched: usize,
    /// Recorded artefacts that were missing or modified, in record
    /// order.
    pub divergences: Vec<Divergence>,
}

impl VerifyReport {
    /// Recorded artefacts this pass examined.
    #[must_use]
    pub fn checked(&self) -> usize {
        self.matched + self.divergences.len()
    }

    /// Whether the tree matched its record.
    #[must_use]
    pub fn is_clean(&self) -> bool {
        self.divergences.is_empty()
    }
}

/// A file the verification could neither hash nor show absent.
#[derive(Debug, thiserror::Error)]
#[error("verify-read {}: {source}", path.display())]
pub struct VerifyReadError {
    /// The file that could not be read.
    pub path: PathBuf,
    /// The underlying failure.
    #[source]
    pub source: std::io::Error,
}

/// Re-hash every file `record` lists, plus its RAP when one is
/// installed, against the tree at `entry_dir`.
///
/// An absent entry directory yields one missing divergence per file the
/// record lists, never a clean report over nothing.
///
/// # Errors
///
/// [`VerifyReadError`] for any read failure other than absence.
pub fn verify_record_tree(
    entry_dir: &Path,
    rap_path: Option<&Path>,
    record: &InstallRecord,
) -> Result<VerifyReport, VerifyReadError> {
    let mut report = VerifyReport::default();
    for (rel, expected) in &record.files {
        check_one(&entry_dir.join(rel), *expected, &mut report)?;
    }
    verify_recorded_rap(rap_path, record, &mut report)?;
    Ok(report)
}

/// Hold the record's RAP alone against the live license directory.
///
/// The RAP lives outside the entry directory, so it stays checkable
/// when the tree is gone. An uninstall that resumes after a crash still
/// removes the RAP, and still owes a proof of what it removes.
///
/// # Errors
///
/// [`VerifyReadError`] for any read failure other than absence.
pub fn verify_recorded_rap(
    rap_path: Option<&Path>,
    record: &InstallRecord,
    report: &mut VerifyReport,
) -> Result<(), VerifyReadError> {
    // An absent RAP is not this tree's divergence: `--keep-rap` and a
    // second title sharing it both leave it absent legitimately.
    if let (Some(rap_path), Some(rap)) = (rap_path, &record.rap) {
        if let Some(bytes) = read_if_present(rap_path)? {
            tally(rap_path, rap.sha256, sha256_of(&bytes), report);
        }
    }
    Ok(())
}

fn read_if_present(path: &Path) -> Result<Option<Vec<u8>>, VerifyReadError> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(source) => Err(VerifyReadError {
            path: path.to_path_buf(),
            source,
        }),
    }
}

/// Hash one recorded artefact into `report`.
fn check_one(
    path: &Path,
    expected: HexSha256,
    report: &mut VerifyReport,
) -> Result<(), VerifyReadError> {
    // A record holds the empty-bytes hash for any zero-byte entry the
    // container carried. Absence gets its own arm, so a deleted
    // placeholder cannot verify as intact.
    let Some(bytes) = read_if_present(path)? else {
        report.divergences.push(Divergence {
            path: path.to_path_buf(),
            kind: DivergenceKind::Missing,
        });
        return Ok(());
    };
    tally(path, expected, sha256_of(&bytes), report);
    Ok(())
}

/// Count one hashed artefact as a match or name it as modified.
fn tally(path: &Path, expected: HexSha256, found: HexSha256, report: &mut VerifyReport) {
    if found == expected {
        report.matched += 1;
    } else {
        report.divergences.push(Divergence {
            path: path.to_path_buf(),
            kind: DivergenceKind::Modified { expected, found },
        });
    }
}

#[cfg(test)]
#[path = "tests/verify_tests.rs"]
mod tests;
