//! Record-driven game uninstall, the inverse of [`crate::game_install`].
//!
//! The install record is the source of truth for what to remove: its
//! `store_path` names the tree. An optional verify gate re-hashes the
//! live tree against the record -- the destroy analog of the install
//! decrypt-proof -- before anything is touched.
//!
//! # Invariants
//!
//! - A record steers nothing until it is checked against the title the
//!   caller named: the right entry kind, and a `store_path` on the way
//!   to that title's own tree.
//! - The live game directory is renamed to a hidden sibling tombstone
//!   (the atomic point) before RAP, record, and tombstone are removed.
//! - The record is removed *before* the tombstone is deleted, so a
//!   crash mid-teardown leaves at most an orphan `.uninstalling-*`
//!   tombstone -- off the boot path, swept on the next uninstall of
//!   that title -- never a record pointing at a half-deleted tree.

use std::path::{Path, PathBuf};

use crate::game_install::sha256_of;
use crate::manifest::Sha256 as HexSha256;
use crate::store::layout::{tombstone_sibling, Artifact, ArtifactKind, StoreLayout, TitleId};
use crate::store::record::InstallRecord;

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
    /// Number of recorded artefacts that matched, when `verify` was
    /// set: the recorded files plus the recorded RAP when it is on disk.
    pub files_verified: Option<usize>,
    /// Number of recorded artefacts that were missing or modified, over
    /// the same set [`Self::files_verified`] counts. Non-zero only under
    /// `force`, the sole way a divergence gets past the gate.
    pub files_diverged: Option<usize>,
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
    /// Loading the install record failed: bad TOML, or a schema this
    /// build does not read.
    #[error("parse install record: {0}")]
    RecordParse(#[from] crate::store::record::InstallRecordParseError),
    /// The operator-supplied title-id is not usable as a single path
    /// component under the store roots, so it names no record.
    #[error("unsafe title-id {title_id:?}")]
    UnsafeTitleId {
        /// The offending title-id.
        title_id: String,
        /// Which rule it broke.
        #[source]
        source: crate::store::layout::StoreKeyError,
    },
    /// The record filed under this title describes some other kind of
    /// store entry, so nothing here names a title tree to remove.
    #[error("install record for {title_id:?} describes a {} entry, not a title base", kind.as_str())]
    RecordKindMismatch {
        /// The requested title-id.
        title_id: String,
        /// The kind the record declared.
        kind: ArtifactKind,
    },
    /// The record's `store_path` names a tree that is not this title's.
    #[error(
        "install record for {title_id:?} names the tree {store_path:?}, which is not this title's"
    )]
    RecordTreeForeign {
        /// The requested title-id.
        title_id: String,
        /// The `store_path` the record declared.
        store_path: String,
    },
    /// A file the record lists is absent from the live tree; pass
    /// `force` to uninstall anyway.
    #[error("recorded file missing from the installed tree: {}", path.display())]
    RecordedFileMissing {
        /// The absent path.
        path: PathBuf,
    },
    /// A live file's hash diverged from the record (the tree was
    /// modified since install); pass `force` to uninstall anyway.
    #[error("tree modified since install: {} (recorded {}, found {})", path.display(), expected.to_hex(), found.to_hex())]
    TreeModified {
        /// The diverging path.
        path: PathBuf,
        /// Hash the record holds.
        expected: HexSha256,
        /// Hash found on disk.
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

/// Re-hash the live tree (and RAP) against `record`, returning
/// `(matched, diverged)` over every recorded artefact checked -- the
/// recorded files plus the recorded RAP when it is present on disk, so
/// the two tallies sum to what was examined. A divergence is
/// [`GameUninstallError::TreeModified`] or
/// [`GameUninstallError::RecordedFileMissing`] unless `force`; under
/// `force` it still increments `diverged`, so the override reports what
/// it waved through instead of dropping it.
fn verify_against_record(
    game_dir: &Path,
    rap_path: Option<&Path>,
    record: &InstallRecord,
    force: bool,
) -> Result<(usize, usize), GameUninstallError> {
    // Tree already gone: nothing to verify (the idempotent path).
    if !game_dir.exists() {
        return Ok((0, 0));
    }
    let mut verified = 0usize;
    let mut diverged = 0usize;
    for (rel, expected) in &record.files {
        let path = game_dir.join(rel);
        // Absence is its own arm, not the empty-bytes hash: a record
        // holds that hash for any zero-byte entry the container carried,
        // so folding the two together lets a deleted placeholder verify
        // as intact.
        let found = match std::fs::read(&path) {
            Ok(bytes) => sha256_of(&bytes),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                diverged += 1;
                if force {
                    continue;
                }
                return Err(GameUninstallError::RecordedFileMissing { path });
            }
            Err(e) => return Err(uio_err("verify-read", &path)(e)),
        };
        if &found == expected {
            verified += 1;
        } else {
            diverged += 1;
            if !force {
                return Err(GameUninstallError::TreeModified {
                    path,
                    expected: *expected,
                    found,
                });
            }
        }
    }
    if let (Some(rp), Some(rap)) = (rap_path, &record.rap) {
        if rp.exists() {
            let bytes = std::fs::read(rp).map_err(uio_err("verify-read", rp))?;
            let found = sha256_of(&bytes);
            if found == rap.sha256 {
                // The RAP counts in both tallies or neither: counting
                // only its divergence would let `diverged` exceed the
                // entries `verified` was drawn from.
                verified += 1;
            } else {
                diverged += 1;
                if !force {
                    return Err(GameUninstallError::TreeModified {
                        path: rp.to_path_buf(),
                        expected: rap.sha256,
                        found,
                    });
                }
            }
        }
    }
    Ok((verified, diverged))
}

/// Refuse a record that does not describe the title the caller named.
///
/// The parse gate proves `store_path` stays under the VFS root, but not
/// whose tree it names: under that root it is free to name another
/// title's tree, or a whole mount, and it steers the tombstone rename
/// and the `remove_dir_all` below.
fn check_record_describes(
    title_id: &str,
    record: &InstallRecord,
) -> Result<(), GameUninstallError> {
    if record.artifact.kind != ArtifactKind::TitleBase {
        return Err(GameUninstallError::RecordKindMismatch {
            title_id: title_id.to_string(),
            kind: record.artifact.kind,
        });
    }
    // Which component carries the id is the store layout's business;
    // every layout puts it somewhere on the path to a title's own tree.
    if !record
        .artifact
        .store_path
        .split('/')
        .any(|part| part == title_id)
    {
        return Err(GameUninstallError::RecordTreeForeign {
            title_id: title_id.to_string(),
            store_path: record.artifact.store_path.clone(),
        });
    }
    Ok(())
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
    opts: UninstallOptions,
) -> Result<GameUninstallOutcome, GameUninstallError> {
    let key = TitleId::new(title_id).map_err(|source| GameUninstallError::UnsafeTitleId {
        title_id: title_id.to_string(),
        source,
    })?;

    // Load the record (the source of truth for what to remove).
    let layout = StoreLayout::new(output_dir);
    let record_path = layout.record_path(&Artifact::TitleBase { title_id: key });
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
    let record = InstallRecord::parse(&text)?;
    check_record_describes(title_id, &record)?;

    let game_dir = layout.resolve_store_path(&record.artifact.store_path);
    // The RAP is keyed by content id in the live exdata directory,
    // which no store entry owns.
    let rap_path = record
        .rap
        .as_ref()
        .map(|r| layout.live_exdata_dir().join(&r.filename));
    let tombstone = tombstone_sibling(&game_dir);

    // Clear any stale tombstone left by a prior interrupted uninstall.
    remove_dir_if_present(&tombstone)?;

    // Verify-before-destroy gate.
    let (files_verified, files_diverged) = if opts.verify {
        let (ok, bad) = verify_against_record(&game_dir, rap_path.as_deref(), &record, opts.force)?;
        (Some(ok), Some(bad))
    } else {
        (None, None)
    };

    // Tombstone rename: the atomic point. Absent tree is idempotent.
    if game_dir.exists() {
        std::fs::rename(&game_dir, &tombstone).map_err(uio_err("rename", &game_dir))?;
    }

    // RAP: remove after the tombstone rename, unless asked to keep it.
    // An already-absent RAP is not reported as removed: the outcome
    // names what this call actually took away.
    let rap_removed = if opts.keep_rap {
        None
    } else if let Some(rp) = &rap_path {
        match std::fs::remove_file(rp) {
            Ok(()) => Some(rp.clone()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(uio_err("remove", rp)(e)),
        }
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
        files_diverged,
    })
}

#[cfg(test)]
#[path = "tests/game_uninstall_tests.rs"]
mod tests;
