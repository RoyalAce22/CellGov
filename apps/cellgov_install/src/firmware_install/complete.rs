//! The completion pass: adds the kernel and the hypervisor a PUP carries
//! to a firmware entry the store already holds, and leaves the entry's
//! tree as it is.
//!
//! A fresh install (`install_pup`, under the `decrypt` feature) stores the kernel with the
//! `dev_flash` tree. This pass serves an entry that already exists: it
//! opens one package of the same PUP and writes the two SELFs beside
//! `dev_flash/`. The pass takes only the PUP the entry came from; the
//! record's source digest says which.

#![cfg_attr(
    not(feature = "decrypt"),
    allow(
        dead_code,
        unused_imports,
        reason = "the completion pass is gated; the feature-on build lints these"
    )
)]

use std::path::{Path, PathBuf};

use super::core_os::{self, CoreOsUnpack};
use super::error::FirmwareInstallError;
use super::install::{dir_non_empty, installed_record, update_files_payload};
use crate::keys::KeyVault;
use crate::manifest;
use crate::progress::{FirmwarePhase, ProgressSink};
use crate::store::layout::{Artifact, StoreLayout, VersionKey};
use crate::store::lock::lock_artifact;
use crate::store::record::{CoreOsRecord, KernelRecord};
use crate::{pup, tar};

/// What a completion pass did with one CoreOS SELF: the kernel or the
/// hypervisor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SelfCompletion {
    /// The pass wrote the SELF into an entry that held none.
    Written,
    /// The pass wrote the SELF over the one the entry held.
    Replaced,
    /// The pass wrote no SELF, and the entry held one. The pass wrote
    /// nothing over that file, and the record still names it.
    Kept {
        /// Why the pass wrote no SELF.
        reason: String,
    },
    /// The pass wrote no SELF, and the entry held none. The record's
    /// block says why, when it names a reason.
    Absent,
}

impl SelfCompletion {
    /// Whether the pass wrote the SELF.
    #[must_use]
    pub fn written(&self) -> bool {
        matches!(self, Self::Written | Self::Replaced)
    }
}

/// What a completion pass produced.
#[derive(Debug, Clone)]
pub struct KernelCompletionOutcome {
    /// The version the PUP's `version.txt` names, and the entry it
    /// completed.
    pub version: String,
    /// The entry directory the kernel landed in.
    pub entry_dir: PathBuf,
    /// The rewritten install record.
    pub record_path: PathBuf,
    /// The block the record now carries.
    pub core_os: CoreOsRecord,
    /// What the pass did with the kernel.
    pub kernel: SelfCompletion,
    /// What the pass did with the hypervisor.
    pub lv1: SelfCompletion,
}

/// Settle the `fresh` unpack over `held`, the block the entry carried.
///
/// Returns the block to record, then what the pass did with the kernel
/// and with the hypervisor. For each SELF that `fresh` did not write and
/// `held` names, the block keeps the held [`KernelRecord`] and drops the
/// fresh omission. The block keeps the held file table when `fresh` read
/// none.
fn settle_over_held(
    held: Option<&CoreOsRecord>,
    fresh: CoreOsUnpack,
) -> (CoreOsRecord, SelfCompletion, SelfCompletion) {
    let kernel_absence = fresh.kernel_absence();
    let lv1_absence = fresh.lv1_absence();
    let mut block = fresh.into_record();
    let kernel = settle_one(
        &mut block.kernel,
        &mut block.omission,
        held.and_then(|h| h.kernel.as_ref()),
        kernel_absence,
    );
    let lv1 = settle_one(
        &mut block.lv1,
        &mut block.lv1_omission,
        held.and_then(|h| h.lv1.as_ref()),
        lv1_absence,
    );
    if block.files.is_empty() {
        if let Some(held) = held {
            block.files.clone_from(&held.files);
        }
    }
    (block, kernel, lv1)
}

/// Settle `stored` and `omission` of one SELF against `held`.
/// `absence` is why the pass wrote no SELF, or `None` when it wrote one.
fn settle_one(
    stored: &mut Option<KernelRecord>,
    omission: &mut Option<String>,
    held: Option<&KernelRecord>,
    absence: Option<String>,
) -> SelfCompletion {
    match (absence, held) {
        (None, Some(_)) => SelfCompletion::Replaced,
        (None, None) => SelfCompletion::Written,
        (Some(reason), Some(held)) => {
            *stored = Some(held.clone());
            *omission = None;
            SelfCompletion::Kept { reason }
        }
        (Some(_), None) => SelfCompletion::Absent,
    }
}

/// Unpack the kernel out of `pup_data` into the installed entry of the
/// version it names; `dev_flash/` stays as it is.
///
/// # Errors
///
/// - [`FirmwareInstallError::NotInstalled`] when the version has no
///   record.
/// - [`FirmwareInstallError::RecordTreeForeign`] when the record names a
///   tree that is not this version's entry.
/// - [`FirmwareInstallError::CompletionPupMismatch`] when the entry was
///   installed from a different PUP.
/// - [`FirmwareInstallError::EntryTreeAbsent`] when the record names a
///   tree that is not there.
/// - [`FirmwareInstallError::Pup`] for a PUP that does not parse, fails
///   its HMAC check, or names no version.
/// - The container, lock, record, and write refusals a full install
///   shares.
#[cfg(feature = "decrypt")]
pub fn complete_kernel(
    pup_data: &[u8],
    keys: &KeyVault,
    output_dir: &Path,
    progress: &dyn ProgressSink,
) -> Result<KernelCompletionOutcome, FirmwareInstallError> {
    crate::store::pre_store::preflight(output_dir)?;
    progress.phase(FirmwarePhase::Reading.code());
    let pup = pup::parse(pup_data)?;
    let pup_sha256 = manifest::Sha256(manifest::sha256_of(pup_data));
    progress.phase(FirmwarePhase::ValidatingHmac.code());
    pup::validate_hashes(pup_data, &pup, keys)?;
    let version = pup::version_key(pup_data, &pup)?;

    let layout = StoreLayout::new(output_dir);
    let artifact = Artifact::Firmware {
        version: VersionKey::new(&version)?,
    };
    let _lock = lock_artifact(&layout, &artifact)?;
    let Some(mut record) = installed_record(output_dir, &version)? else {
        return Err(FirmwareInstallError::NotInstalled { version });
    };
    // The parse gate proves only that `store_path` stays under the VFS
    // root. The lock held is this version's, so a write down a foreign
    // path would land in an entry another record owns.
    // `firmware_uninstall::check_record_describes` refuses the same for
    // a removal.
    let entry_dir = layout.resolve_store_path(&record.artifact.store_path);
    if entry_dir != layout.entry_dir(&artifact) {
        return Err(FirmwareInstallError::RecordTreeForeign {
            version,
            store_path: record.artifact.store_path,
        });
    }
    if record.source.sha256 != pup_sha256 {
        return Err(FirmwareInstallError::CompletionPupMismatch {
            version,
            installed: record.source.sha256,
            incoming: pup_sha256,
        });
    }
    if !dir_non_empty(&entry_dir)? {
        return Err(FirmwareInstallError::EntryTreeAbsent {
            version,
            path: entry_dir,
        });
    }

    let update_data = update_files_payload(pup_data, &pup)?;
    let outer = tar::parse(update_data).map_err(FirmwareInstallError::OuterTar)?;
    progress.phase(FirmwarePhase::UnpackingKernel.code());
    let fresh = core_os::unpack(&outer, &entry_dir, keys);
    let (block, kernel, lv1) = settle_over_held(record.core_os.as_ref(), fresh);
    record.core_os = Some(block.clone());

    let record_path = layout.record_path(&artifact);
    let text = record.to_toml()?;
    // The old record is valid; a truncate-then-write that stops short
    // would leave the entry unreadable.
    core_os::write_whole(&record_path, text.as_bytes()).map_err(|source| {
        FirmwareInstallError::Io {
            op: "write",
            path: record_path.clone(),
            source,
        }
    })?;
    progress.finished();
    Ok(KernelCompletionOutcome {
        version,
        entry_dir,
        record_path,
        core_os: block,
        kernel,
        lv1,
    })
}

#[cfg(all(test, feature = "decrypt"))]
#[path = "tests/complete_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/settle_over_held_tests.rs"]
mod settle_over_held_tests;

#[cfg(all(test, feature = "decrypt"))]
#[path = "tests/complete_rerun_tests.rs"]
mod complete_rerun_tests;
