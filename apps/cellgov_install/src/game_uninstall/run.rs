//! Executing an [`UninstallPlan`]: the verify gate, then the
//! tombstone-first teardown of each entry.

use std::path::{Path, PathBuf};

use crate::store::layout::tombstone_sibling;
use crate::store::record::InstallRecord;
use crate::store::verify::{verify_record_tree, verify_recorded_rap, DivergenceKind, VerifyReport};

use super::error::{uio_err, GameUninstallError};
use super::plan::{plan, EntryVersion, UninstallPlan, UninstallScope};

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

/// One store entry an [`uninstall`] removed.
#[derive(Debug, Clone)]
pub struct RemovedEntry {
    /// Which entry of the title this was.
    pub version: EntryVersion,
    /// The tree that was removed (or that was already gone).
    pub tree_removed: PathBuf,
    /// The install record removed with it, or the one a prior
    /// interrupted run already took.
    pub record_removed: PathBuf,
}

/// What an [`uninstall`] removed.
#[derive(Debug, Clone)]
pub struct GameUninstallOutcome {
    /// The uninstalled title-id.
    pub title_id: String,
    /// The entries removed, in removal order.
    pub removed: Vec<RemovedEntry>,
    /// The RAP removed from `exdata/`, if any.
    pub rap_removed: Option<PathBuf>,
    /// Installed update versions the scope left in place.
    pub kept_updates: Vec<String>,
    /// Number of recorded artefacts that matched, when `verify` was set.
    ///
    /// The gate examines the recorded files of every removed entry, plus
    /// the recorded RAP when it is on disk, less any entry whose tree
    /// was already gone. A shortfall against
    /// [`UninstallPlan::recorded_files`] names the entries that were not
    /// there to check.
    pub files_verified: Option<usize>,
    /// Number of recorded artefacts that were missing or modified, over
    /// the same set [`Self::files_verified`] counts. Non-zero only under
    /// `force`, the sole way a divergence passes the gate.
    pub files_diverged: Option<usize>,
}

impl GameUninstallOutcome {
    /// The base entry, when the scope removed one.
    #[must_use]
    pub fn base(&self) -> Option<&RemovedEntry> {
        self.removed
            .iter()
            .find(|e| e.version == EntryVersion::Base)
    }
}

fn remove_dir_if_present(path: &Path) -> Result<(), GameUninstallError> {
    match std::fs::remove_dir_all(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(uio_err("remove", path)(e)),
    }
}

/// Re-hash one entry's tree, and for the base its RAP, against the
/// record.
///
/// Returns `(matched, diverged)` over every recorded artefact checked,
/// so the two tallies sum to what the gate examined.
///
/// A divergence is [`GameUninstallError::TreeModified`] or
/// [`GameUninstallError::RecordedFileMissing`] unless `force`. Under
/// `force` it still counts in `diverged`.
///
/// A tree a crash already removed leaves only its RAP to check, so a
/// re-run clears that state without `force`.
fn verify_against_record(
    tree_dir: &Path,
    rap_path: Option<&Path>,
    record: &InstallRecord,
    force: bool,
) -> Result<(usize, usize), GameUninstallError> {
    let report = if std::fs::exists(tree_dir).map_err(uio_err("stat", tree_dir))? {
        verify_record_tree(tree_dir, rap_path, record)?
    } else {
        let mut report = VerifyReport::default();
        verify_recorded_rap(rap_path, record, &mut report)?;
        report
    };
    if !force {
        if let Some(first) = report.divergences.first() {
            return Err(match first.kind {
                DivergenceKind::Missing => GameUninstallError::RecordedFileMissing {
                    path: first.path.clone(),
                },
                DivergenceKind::Modified { expected, found } => GameUninstallError::TreeModified {
                    path: first.path.clone(),
                    expected,
                    found,
                },
            });
        }
    }
    Ok((report.matched, report.divergences.len()))
}

/// Remove the entries `scope` names. See the module invariants for the
/// rename-then-teardown ordering.
///
/// Idempotent: an entry whose tree is already gone still removes the
/// RAP and record and succeeds; a title-id with no record is
/// [`GameUninstallError::NoRecord`], never a silent success.
///
/// # Errors
///
/// Every [`plan`] refusal, since `uninstall` resolves the scope before
/// it touches anything, plus the verify and filesystem failures.
pub fn uninstall(
    title_id: &str,
    output_dir: &Path,
    scope: &UninstallScope,
    opts: UninstallOptions,
) -> Result<GameUninstallOutcome, GameUninstallError> {
    execute(&plan(title_id, output_dir, scope)?, opts)
}

/// Run a plan that [`plan`] already resolved.
///
/// The whole verify gate runs before the first rename, so a divergence
/// in the last entry still leaves every earlier one in place.
///
/// # Errors
///
/// The verify and filesystem failures.
pub fn execute(
    plan: &UninstallPlan,
    opts: UninstallOptions,
) -> Result<GameUninstallOutcome, GameUninstallError> {
    let mut verified = 0usize;
    let mut diverged = 0usize;
    if opts.verify {
        for entry in &plan.entries {
            let rap = match entry.version {
                EntryVersion::Base => plan.rap.as_deref(),
                EntryVersion::Update(_) => None,
            };
            let (ok, bad) = verify_against_record(&entry.tree_dir, rap, &entry.record, opts.force)?;
            verified += ok;
            diverged += bad;
        }
    }

    let mut removed = Vec::with_capacity(plan.entries.len());
    let mut rap_removed = None;
    for entry in &plan.entries {
        let tombstone = tombstone_sibling(&entry.tree_dir);
        // Clear any stale tombstone left by a prior interrupted
        // uninstall.
        remove_dir_if_present(&tombstone)?;

        // Tombstone rename: the atomic point. An absent tree is
        // idempotent; a stat that fails refuses here, since the record
        // removal below would otherwise leave the tree unnamed.
        if std::fs::exists(&entry.tree_dir).map_err(uio_err("stat", &entry.tree_dir))? {
            std::fs::rename(&entry.tree_dir, &tombstone)
                .map_err(uio_err("rename", &entry.tree_dir))?;
        }

        // RAP: removed with the base, after its tombstone rename, unless
        // `keep_rap`. An already-absent RAP is not reported as removed:
        // the outcome names what this call removed.
        if entry.version == EntryVersion::Base && !opts.keep_rap {
            if let Some(rap) = &plan.rap {
                match std::fs::remove_file(rap) {
                    Ok(()) => rap_removed = Some(rap.clone()),
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                    Err(e) => return Err(uio_err("remove", rap)(e)),
                }
            }
        }

        // Record before tombstone (see the module ordering).
        match std::fs::remove_file(&entry.record_path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(uio_err("remove", &entry.record_path)(e)),
        }

        // Tombstone last. An orphan tombstone is acceptable residue.
        remove_dir_if_present(&tombstone)?;

        removed.push(RemovedEntry {
            version: entry.version.clone(),
            tree_removed: entry.tree_dir.clone(),
            record_removed: entry.record_path.clone(),
        });
    }

    Ok(GameUninstallOutcome {
        title_id: plan.title_id.clone(),
        removed,
        rap_removed,
        kept_updates: plan.kept_updates.clone(),
        files_verified: opts.verify.then_some(verified),
        files_diverged: opts.verify.then_some(diverged),
    })
}

#[cfg(test)]
#[path = "tests/run_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/scope_tests.rs"]
mod scope_tests;
