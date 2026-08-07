//! Record-driven game uninstall, the inverse of [`crate::game_install`].
//!
//! The install record (`installs/<title-id>.install.toml`) is the
//! source of truth for what to remove. An optional verify gate
//! re-hashes the live tree against the record -- the destroy analog of
//! the install decrypt-proof -- before anything is touched.
//!
//! # Invariants
//!
//! - The live game directory is renamed to a `.uninstalling-<title-id>`
//!   tombstone (the atomic point) before RAP, record, and tombstone are
//!   removed.
//! - The record is removed *before* the tombstone is deleted, so a
//!   crash mid-teardown leaves at most an orphan `.uninstalling-*`
//!   tombstone -- off the boot path, swept on the next uninstall of
//!   that title -- never a record pointing at a half-deleted tree.

use std::path::{Path, PathBuf};

use crate::game_install::{sha256_of, InstallRecord, HDD0_USER};
use crate::manifest::Sha256 as HexSha256;

/// Options for [`uninstall`].
#[derive(Debug, Clone, Copy)]
pub struct UninstallOptions {
    /// Re-hash the live tree (and RAP) against the record before
    /// removing anything; the destroy analog of the install
    /// decrypt-proof.
    pub verify: bool,
    /// Leave the RAP in `exdata/` (another title may share it).
    pub keep_rap: bool,
    /// Proceed even if `verify` finds a modified tree.
    pub force: bool,
}

/// What an [`uninstall`] removed.
#[derive(Debug, Clone)]
pub struct GameUninstallOutcome {
    /// The uninstalled title-id.
    pub title_id: String,
    /// The game directory that was removed (or that was already gone).
    pub game_dir_removed: PathBuf,
    /// The RAP removed from `exdata/`, if any.
    pub rap_removed: Option<PathBuf>,
    /// The install record that was removed.
    pub record_removed: PathBuf,
    /// Number of recorded files verified, when `verify` was set.
    pub files_verified: Option<usize>,
}

/// Why an uninstall failed. Local to this operation per the
/// per-operation error rule.
#[derive(Debug, thiserror::Error)]
pub enum GameUninstallError {
    /// No install record exists for the title; nothing to uninstall.
    #[error("no install record for title {title_id:?}")]
    NoRecord {
        /// The requested title-id.
        title_id: String,
    },
    /// Reading the install record failed (for a reason other than absence).
    #[error("read install record {}: {source}", path.display())]
    RecordRead {
        /// The record path.
        path: PathBuf,
        /// Underlying error.
        #[source]
        source: std::io::Error,
    },
    /// Parsing the install record failed.
    #[error("parse install record: {0}")]
    RecordParse(#[from] toml::de::Error),
    /// A live file's hash diverged from the record (the tree was
    /// modified since install); pass `force` to uninstall anyway.
    #[error("tree modified since install: {} (recorded {}, found {})", path.display(), expected.to_hex(), found.to_hex())]
    TreeModified {
        /// The diverging path.
        path: PathBuf,
        /// Hash the record holds.
        expected: HexSha256,
        /// Hash found on disk (the empty-bytes hash if the file is gone).
        found: HexSha256,
    },
    /// A filesystem operation failed.
    #[error("{op} {}: {source}", path.display())]
    Io {
        /// What was being attempted.
        op: &'static str,
        /// The path involved.
        path: PathBuf,
        /// Underlying error.
        #[source]
        source: std::io::Error,
    },
}

fn uio_err<'a>(
    op: &'static str,
    path: &'a Path,
) -> impl Fn(std::io::Error) -> GameUninstallError + 'a {
    move |source| GameUninstallError::Io {
        op,
        path: path.to_path_buf(),
        source,
    }
}

/// Remove a directory tree, tolerating its absence.
fn remove_dir_if_present(path: &Path) -> Result<(), GameUninstallError> {
    match std::fs::remove_dir_all(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(uio_err("remove", path)(e)),
    }
}

/// Re-hash the live tree (and RAP) against `record`, returning the
/// count of recorded files verified. A divergence is
/// [`GameUninstallError::TreeModified`] unless `force`.
fn verify_against_record(
    game_dir: &Path,
    rap_path: Option<&Path>,
    record: &InstallRecord,
    force: bool,
) -> Result<usize, GameUninstallError> {
    // Tree already gone: nothing to verify (the idempotent path).
    if !game_dir.exists() {
        return Ok(0);
    }
    let mut verified = 0usize;
    for (rel, expected) in &record.files {
        let path = game_dir.join(rel);
        let found = match std::fs::read(&path) {
            Ok(bytes) => sha256_of(&bytes),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => sha256_of(&[]),
            Err(e) => return Err(uio_err("verify-read", &path)(e)),
        };
        if &found == expected {
            verified += 1;
        } else if !force {
            return Err(GameUninstallError::TreeModified {
                path,
                expected: *expected,
                found,
            });
        }
    }
    if let (Some(rp), Some(rap)) = (rap_path, &record.rap) {
        if rp.exists() {
            let bytes = std::fs::read(rp).map_err(uio_err("verify-read", rp))?;
            let found = sha256_of(&bytes);
            if found != rap.sha256 && !force {
                return Err(GameUninstallError::TreeModified {
                    path: rp.to_path_buf(),
                    expected: rap.sha256,
                    found,
                });
            }
        }
    }
    Ok(verified)
}

/// Remove an installed title named by its record. See the module
/// invariants for the rename-then-teardown ordering.
///
/// Idempotent: a record whose tree is already gone still removes the
/// RAP and record and succeeds; a title-id with no record is
/// [`GameUninstallError::NoRecord`], never a silent success.
pub fn uninstall(
    title_id: &str,
    output_dir: &Path,
    installs_dir: &Path,
    opts: UninstallOptions,
) -> Result<GameUninstallOutcome, GameUninstallError> {
    // Load the record (the source of truth for what to remove).
    let record_path = installs_dir.join(format!("{title_id}.install.toml"));
    let text = match std::fs::read_to_string(&record_path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(GameUninstallError::NoRecord {
                title_id: title_id.to_string(),
            })
        }
        Err(source) => {
            return Err(GameUninstallError::RecordRead {
                path: record_path,
                source,
            })
        }
    };
    let record: InstallRecord = toml::from_str(&text)?;

    // Resolve targets from the recorded distribution / source kind.
    let dev_hdd0 = output_dir.join("dev_hdd0");
    let is_disc = record.title.distribution == "disc-iso" || record.source.kind == "iso";
    let (game_dir, rap_path) = if is_disc {
        (output_dir.join("dev_bdvd").join(title_id), None)
    } else {
        let exdata = dev_hdd0.join("home").join(HDD0_USER).join("exdata");
        let rap = record.rap.as_ref().map(|r| exdata.join(&r.filename));
        (dev_hdd0.join("game").join(title_id), rap)
    };
    let tombstone = game_dir.with_file_name(format!(".uninstalling-{title_id}"));

    // Clear any stale tombstone left by a prior interrupted uninstall.
    remove_dir_if_present(&tombstone)?;

    // Verify-before-destroy gate.
    let files_verified = if opts.verify {
        Some(verify_against_record(
            &game_dir,
            rap_path.as_deref(),
            &record,
            opts.force,
        )?)
    } else {
        None
    };

    // Tombstone rename: the atomic point. Absent tree is idempotent.
    if game_dir.exists() {
        std::fs::rename(&game_dir, &tombstone).map_err(uio_err("rename", &game_dir))?;
    }

    // RAP: remove after the tombstone rename, unless asked to keep it.
    let rap_removed = if opts.keep_rap {
        None
    } else if let Some(rp) = &rap_path {
        match std::fs::remove_file(rp) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(uio_err("remove", rp)(e)),
        }
        Some(rp.clone())
    } else {
        None
    };

    // Record before tombstone (see Ordering).
    match std::fs::remove_file(&record_path) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(uio_err("remove", &record_path)(e)),
    }

    // Tombstone last. An orphan tombstone is acceptable residue.
    remove_dir_if_present(&tombstone)?;

    Ok(GameUninstallOutcome {
        title_id: title_id.to_string(),
        game_dir_removed: game_dir,
        rap_removed,
        record_removed: record_path,
        files_verified,
    })
}

#[cfg(test)]
#[path = "tests/game_uninstall_tests.rs"]
mod tests;
