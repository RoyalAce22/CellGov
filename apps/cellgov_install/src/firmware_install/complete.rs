//! The completion pass: adds the kernel a PUP carries to a firmware
//! entry the store already holds, and leaves the entry's tree as it is.
//!
//! A fresh install ([`super::install_pup`]) stores the kernel with the
//! `dev_flash` tree. This pass serves an entry that already exists: it
//! opens one package of the same PUP and writes one file beside
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

use super::core_os;
use super::error::FirmwareInstallError;
use super::install::{dir_non_empty, installed_record, update_files_payload};
use crate::keys::KeyVault;
use crate::manifest;
use crate::progress::{FirmwarePhase, ProgressSink};
use crate::store::layout::{Artifact, StoreLayout, VersionKey};
use crate::store::lock::lock_artifact;
use crate::store::record::CoreOsRecord;
use crate::{pup, tar};

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
    /// Whether this pass wrote over a kernel the entry already held.
    pub replaced: bool,
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
    let held_kernel = record
        .core_os
        .as_ref()
        .is_some_and(|block| block.kernel.is_some());
    let block = core_os::unpack(&outer, &entry_dir, keys).into_record();
    // An omission writes nothing over the file that was there.
    let replaced = held_kernel && block.kernel.is_some();
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
        replaced,
    })
}

#[cfg(all(test, feature = "decrypt"))]
#[path = "tests/complete_tests.rs"]
mod tests;
