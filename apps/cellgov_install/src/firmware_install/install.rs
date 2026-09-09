//! The firmware installer: one PUP becomes one store entry at
//! `firmware/<version>/`, keyed by the version the extracted tree
//! names.
//!
//! # Invariants
//!
//! - Nothing is written outside
//!   [`StoreLayout::firmware_staging_dir`](crate::store::StoreLayout::firmware_staging_dir)
//!   until the commit, so a fault before it -- including the version
//!   gate refusing, which cannot run until the tree is extracted --
//!   discards the whole install.
//! - The commit is a fixed sequence:
//!   1. remove the record the entry already has,
//!   2. rename the staged tree onto the entry directory -- the commit
//!      point,
//!   3. write the new record.
//!
//!   A record therefore never points at an absent tree. A fault leaves
//!   either an unrecorded tree, which the version gate refuses on its
//!   own terms, or no tree at all, which it reads as free. Nothing
//!   syncs a directory, so a host crash can still reorder the unlink
//!   and the rename.
//! - A commit that fails leaves the staged tree where it is rather than
//!   discarding it: the next install of any version sweeps it by name,
//!   and re-extracting a PUP costs minutes.
//! - The commit rename goes through [`rename_with_retry`]; an on-access
//!   scanner opens the tree the installer just wrote.

#![cfg_attr(
    not(feature = "decrypt"),
    allow(
        dead_code,
        unused_imports,
        reason = "the firmware installer is gated; the feature-on build lints these"
    )
)]

use std::path::{Path, PathBuf};

use cellgov_ps3_abi::dev_flash::FLASH_MOUNT;
use cellgov_ps3_abi::pup::ENTRY_ID_UPDATE_FILES;

use super::error::{FirmwareInstallError, PackageFailure};
#[cfg(feature = "decrypt")]
use super::manifest_build::build_manifest;
use super::manifest_build::ManifestOmission;
use super::prune::is_install_excluded;
use super::version_txt::read_version;
use crate::keys::KeyVault;
use crate::manifest::{self, serialize_manifest, MANIFEST_FILE};
use crate::progress::{FirmwarePhase, ProgressSink};
use crate::store::layout::{Artifact, ArtifactKind, StoreLayout, VersionKey};
use crate::store::lock::{lock_artifact, lock_firmware_staging};
use crate::store::record::{
    ArtifactRecord, InstallRecord, InstallRecordParseError, SourceRecord,
    INSTALL_RECORD_FORMAT_VERSION,
};
use crate::store::rename::rename_with_retry;
use crate::{pup, sce, tar};

/// `source.kind` every firmware record carries.
const FIRMWARE_SOURCE_KIND: &str = "pup";

/// Outer-TAR name fragment marking a dev_flash payload package.
///
/// A retail PUP's `update_files` TAR names its flash-1 payloads
/// `dev_flash_NNN.tar` and carries the revocation list beside them as
/// `dev_flash3_NNN.tar`. The trailing underscore drops that
/// revocation-list package.
const DEV_FLASH_PACKAGE: &str = "dev_flash_";

/// The `update_files` payload: the TAR of SCE-wrapped dev_flash
/// packages.
///
/// The extent is bounds-checked here, so an entry whose declared extent
/// leaves the buffer is distinguished from an absent entry.
fn update_files_payload<'a>(
    pup_data: &'a [u8],
    pup: &pup::Pup,
) -> Result<&'a [u8], FirmwareInstallError> {
    let entry = pup
        .entries
        .iter()
        .find(|e| e.entry_id == ENTRY_ID_UPDATE_FILES)
        .ok_or(FirmwareInstallError::NoUpdateFiles)?;
    usize::try_from(entry.data_offset)
        .ok()
        .zip(usize::try_from(entry.data_length).ok())
        .and_then(|(start, len)| pup_data.get(start..)?.get(..len))
        .ok_or(FirmwareInstallError::UpdateFilesOutOfBounds {
            offset: entry.data_offset,
            length: entry.data_length,
            file_len: pup_data.len(),
        })
}

/// The outer TAR's dev_flash payload packages, in archive order.
///
/// An outer TAR with none of them names no firmware tree, so this
/// refuses the install instead of staging zero files.
fn dev_flash_packages(
    outer: &[tar::TarEntry],
) -> Result<Vec<&tar::TarEntry>, FirmwareInstallError> {
    let packages: Vec<&tar::TarEntry> = outer
        .iter()
        .filter(|e| e.name.contains(DEV_FLASH_PACKAGE))
        .collect();
    if packages.is_empty() {
        return Err(FirmwareInstallError::NoDevFlashPackages);
    }
    Ok(packages)
}

/// What one dev_flash package contributed to the staged tree.
#[derive(Debug, Clone)]
pub struct PackageSummary {
    /// Package name, without its outer-TAR directory prefix.
    pub package: String,
    /// Files written to the staging tree.
    pub written: usize,
    /// Entries the emulator prune dropped.
    pub pruned: usize,
    /// Entries whose name addressed no file.
    pub skipped: usize,
}

/// What a completed firmware install produced, for the CLI to report.
#[derive(Debug, Clone)]
pub struct FirmwareInstallOutcome {
    /// The version key read from `vsh/etc/version.txt`.
    pub version: String,
    /// The committed entry directory, holding the mount trees.
    pub entry_dir: PathBuf,
    /// The written `firmware.toml`.
    pub manifest_path: PathBuf,
    /// Modules the manifest covers.
    pub manifest_entries: usize,
    /// Modules the manifest could not cover, and why.
    pub omissions: Vec<ManifestOmission>,
    /// Per-package tallies, in extraction order.
    pub packages: Vec<PackageSummary>,
    /// Files the packages extracted into the tree, not counting the
    /// `firmware.toml` written over them.
    pub files: usize,
    /// The written install record.
    pub record_path: PathBuf,
    /// Whether `--force` replaced an already-installed version.
    pub replaced: bool,
    /// Refusals the commit rename outwaited before it landed. Non-zero
    /// means a handle was open under the staged tree, typically an
    /// on-access scanner's.
    pub rename_retries: u32,
}

fn io_err<'a>(
    op: &'static str,
    path: &'a Path,
) -> impl Fn(std::io::Error) -> FirmwareInstallError + 'a {
    move |source| FirmwareInstallError::Io {
        op,
        path: path.to_path_buf(),
        source,
    }
}

/// Clear and recreate the staging directory, so no residue from an
/// interrupted install survives into the commit rename.
fn prepare_staging(
    staging_dir: &Path,
    progress: &dyn ProgressSink,
) -> Result<(), FirmwareInstallError> {
    // Removing an interrupted install's multi-gigabyte tree takes long
    // enough that, unannounced, it reads as a stall.
    if staging_dir.exists() {
        progress.phase(FirmwarePhase::ClearingStaging.code());
    }
    match std::fs::remove_dir_all(staging_dir) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(io_err("remove", staging_dir)(e)),
    }
    std::fs::create_dir_all(staging_dir).map_err(io_err("create dir", staging_dir))
}

/// Run `f`, removing the staging directory if it fails.
///
/// A cleanup that cannot remove the root falsifies the module's
/// discard-whole invariant, so it surfaces as
/// [`FirmwareInstallError::StagingResidue`] carrying the original fault.
fn run_or_clean<T>(
    staging_dir: &Path,
    f: impl FnOnce() -> Result<T, FirmwareInstallError>,
) -> Result<T, FirmwareInstallError> {
    match f() {
        Ok(v) => Ok(v),
        Err(e) => match std::fs::remove_dir_all(staging_dir) {
            Ok(()) => Err(e),
            Err(c) if c.kind() == std::io::ErrorKind::NotFound => Err(e),
            Err(c) => Err(FirmwareInstallError::StagingResidue {
                path: staging_dir.to_path_buf(),
                source: c,
                cause: Box::new(e),
            }),
        },
    }
}

/// Whether `path` exists and contains at least one entry.
///
/// A stat that fails is not an absent directory: it refuses here
/// rather than reporting an occupied target as free.
fn dir_non_empty(path: &Path) -> Result<bool, FirmwareInstallError> {
    if !std::fs::exists(path).map_err(io_err("stat", path))? {
        return Ok(false);
    }
    let mut entries = std::fs::read_dir(path).map_err(io_err("read dir", path))?;
    Ok(entries.next().is_some())
}

/// Read an install record, distinguishing absence from a record that is
/// there and unreadable.
fn read_record(path: &Path) -> Result<Option<InstallRecord>, FirmwareInstallError> {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(io_err("read", path)(e)),
    };
    InstallRecord::parse(&text)
        .map(Some)
        .map_err(
            |source: InstallRecordParseError| FirmwareInstallError::RecordParse {
                path: path.to_path_buf(),
                source: Box::new(source),
            },
        )
}

/// The record of the firmware entry keyed `version`, when one is
/// installed.
///
/// Records are the store's index: an unrecorded tree under the entry
/// directory is residue and reads as not installed.
///
/// # Errors
///
/// - [`FirmwareInstallError::StoreKey`] when `version` is not a store
///   directory name.
/// - [`FirmwareInstallError::Io`] for a record that is there and cannot
///   be read.
/// - [`FirmwareInstallError::RecordParse`] for a record this build
///   refuses.
/// - [`FirmwareInstallError::RecordMismatch`] for a record under that
///   key that describes something else.
pub fn installed_record(
    output_dir: &Path,
    version: &str,
) -> Result<Option<InstallRecord>, FirmwareInstallError> {
    let artifact = Artifact::Firmware {
        version: VersionKey::new(version)?,
    };
    let record_path = StoreLayout::new(output_dir).record_path(&artifact);
    let Some(existing) = read_record(&record_path)? else {
        return Ok(None);
    };
    if existing.artifact.kind != ArtifactKind::Firmware || existing.artifact.version != version {
        return Err(FirmwareInstallError::RecordMismatch {
            path: record_path,
            version: version.to_string(),
        });
    }
    Ok(Some(existing))
}

/// Hold the entry this version would commit into.
///
/// Records are the store's index, so an installed version is one with a
/// record; a tree with no record is residue this gate refuses on its
/// own terms. Returns whether an installed version is being replaced.
fn check_entry(
    record_path: &Path,
    entry_dir: &Path,
    version: &str,
    pup_sha256: manifest::Sha256,
    force: bool,
) -> Result<bool, FirmwareInstallError> {
    let Some(existing) = read_record(record_path)? else {
        if dir_non_empty(entry_dir)? && !force {
            return Err(FirmwareInstallError::TargetExists {
                path: entry_dir.to_path_buf(),
            });
        }
        return Ok(false);
    };
    if existing.artifact.kind != ArtifactKind::Firmware || existing.artifact.version != version {
        return Err(FirmwareInstallError::RecordMismatch {
            path: record_path.to_path_buf(),
            version: version.to_string(),
        });
    }
    if !force {
        // One version string over two different images is an operator
        // error to surface, so the two refusals read differently.
        return Err(if existing.source.sha256 == pup_sha256 {
            FirmwareInstallError::VersionInstalled {
                version: version.to_string(),
                pup_sha256,
            }
        } else {
            FirmwareInstallError::VersionInstalledFromAnotherPup {
                version: version.to_string(),
                installed: existing.source.sha256,
                incoming: pup_sha256,
            }
        });
    }
    Ok(true)
}

/// Decrypt and extract every dev_flash package into `staging_root`.
///
/// Package failures are collected rather than short-circuiting, so the
/// caller can name every one at once.
#[cfg(feature = "decrypt")]
fn extract_packages(
    packages: &[&tar::TarEntry],
    staging_root: &Path,
    keys: &KeyVault,
    progress: &dyn ProgressSink,
) -> ExtractTally {
    let mut tally = ExtractTally::default();
    // A determinate bar renders only while the measured phase is
    // current, so the whole loop is one phase.
    progress.phase(FirmwarePhase::Extracting.code());
    for entry in packages {
        let short = entry.name.rsplit('/').next().unwrap_or(&entry.name);
        progress.item_started(short);
        match sce::decrypt_package(&entry.data, keys) {
            Ok(inner) => match tar::parse(&inner) {
                Ok(packaged_files) => {
                    let packaged = packaged_files.len();
                    let inner_files: Vec<tar::TarEntry> = packaged_files
                        .into_iter()
                        .filter(|f| !is_install_excluded(&f.name))
                        .collect();
                    let report = tar::extract_to_disk(&inner_files, staging_root);
                    tally.files += report.written;
                    tally.packages.push(PackageSummary {
                        package: short.to_string(),
                        written: report.written,
                        pruned: packaged - inner_files.len(),
                        skipped: report.skipped,
                    });
                    tally.extract_errors.extend(report.errors);
                }
                Err(source) => tally.failed.push(PackageFailure::InnerTar {
                    package: short.to_string(),
                    source,
                }),
            },
            Err(source) => tally.failed.push(PackageFailure::Decrypt {
                package: short.to_string(),
                source,
            }),
        }
        // The inner file sizes are not known until a package is open,
        // so progress is denominated in the packages' payload bytes.
        progress.advanced(entry.data.len() as u64);
        progress.item_finished();
    }
    tally
}

/// What the extraction loop produced, before it is held to the
/// nothing-partial rule.
#[derive(Debug, Default)]
struct ExtractTally {
    files: usize,
    packages: Vec<PackageSummary>,
    failed: Vec<PackageFailure>,
    extract_errors: Vec<tar::ExtractError>,
}

impl ExtractTally {
    /// Refuse a run that wrote nothing, or that is short of the
    /// firmware the PUP carries.
    ///
    /// A manifest built over a partial tree would record the gap as if
    /// it were the image. Every later run would then compare against a
    /// firmware set short of the PUP's.
    fn into_complete(self, attempted: usize) -> Result<Self, FirmwareInstallError> {
        if !self.failed.is_empty() || !self.extract_errors.is_empty() {
            return Err(FirmwareInstallError::PartialInstall {
                files: self.files,
                packages: attempted,
                packages_failed: self.failed,
                extract_errors: self.extract_errors,
            });
        }
        if self.files == 0 {
            return Err(FirmwareInstallError::ProducedNothing {
                packages: attempted,
            });
        }
        Ok(self)
    }
}

/// Install one PUP into `output_dir`'s versioned store, at
/// `firmware/<version>/`.
///
/// The version cannot be known before the tree is extracted, so the
/// whole install stages first and the duplicate-version gate runs
/// against the staged tree; `force` replaces an installed version whole.
///
/// # Errors
///
/// [`FirmwareInstallError::PreStore`] when the root still holds the
/// pre-store layout. Nothing is read or staged before this refusal.
///
/// [`FirmwareInstallError::NoDevFlashPackages`] when the PUP names no
/// firmware tree, [`FirmwareInstallError::PartialInstall`] when any
/// package failed to land, [`FirmwareInstallError::VersionInstalled`] and
/// [`FirmwareInstallError::VersionInstalledFromAnotherPup`] for a
/// duplicate, [`FirmwareInstallError::TargetExists`] for an unrecorded
/// entry directory that is not empty, and the container / staging /
/// record failures.
///
/// [`FirmwareInstallError::Locked`] comes from either of two claims:
///
/// - the staging directory, claimed before the sweep;
/// - the version's own entry, which the install cannot name until it
///   extracts the tree.
///
/// The second claim therefore refuses only after the whole extraction
/// runs, and the pass discards the staged tree with it.
#[cfg(feature = "decrypt")]
pub fn install_pup(
    pup_data: &[u8],
    keys: &KeyVault,
    output_dir: &Path,
    force: bool,
    progress: &dyn ProgressSink,
) -> Result<FirmwareInstallOutcome, FirmwareInstallError> {
    crate::store::pre_store::preflight(output_dir)?;
    progress.phase(FirmwarePhase::Reading.code());
    let pup = pup::parse(pup_data)?;
    let pup_sha256 = manifest::Sha256(manifest::sha256_of(pup_data));

    progress.phase(FirmwarePhase::ValidatingHmac.code());
    pup::validate_hashes(pup_data, &pup, keys)?;

    let update_data = update_files_payload(pup_data, &pup)?;
    let outer_tar = tar::parse(update_data).map_err(FirmwareInstallError::OuterTar)?;
    let packages = dev_flash_packages(&outer_tar)?;
    progress.totals(
        packages.len(),
        packages.iter().map(|e| e.data.len() as u64).sum(),
    );

    let layout = StoreLayout::new(output_dir);
    // The claim precedes the sweep and holds past the commit rename
    // that moves the staging directory onto the entry it becomes.
    let _staging_lock = lock_firmware_staging(&layout)?;
    let staging_root = layout.firmware_staging_dir();
    prepare_staging(&staging_root, progress)?;

    let (staged, _version_lock) = run_or_clean(&staging_root, || {
        let tally = extract_packages(&packages, &staging_root, keys, progress)
            .into_complete(packages.len())?;

        let dev_flash_dir = staging_root.join(FLASH_MOUNT);
        let version = VersionKey::new(&read_version(&dev_flash_dir)?)?;
        let artifact = Artifact::Firmware {
            version: version.clone(),
        };
        let entry_dir = layout.entry_dir(&artifact);
        let record_path = layout.record_path(&artifact);
        // The install knows the version only here. The claim holds past
        // the commit that fills the entry.
        let version_lock = lock_artifact(&layout, &artifact)?;
        let replaced = check_entry(
            &record_path,
            &entry_dir,
            version.as_str(),
            pup_sha256,
            force,
        )?;

        progress.phase(FirmwarePhase::BuildingManifest.code());
        let (manifest, omissions) = build_manifest(
            pup_sha256,
            pup.image_version,
            &version,
            &dev_flash_dir,
            keys,
        )?;
        let manifest_entries = manifest.files.len();
        let staged_manifest = dev_flash_dir.join(MANIFEST_FILE);
        std::fs::write(&staged_manifest, serialize_manifest(&manifest)?)
            .map_err(io_err("write", &staged_manifest))?;

        let staged = Staged {
            record: InstallRecord {
                format_version: INSTALL_RECORD_FORMAT_VERSION,
                artifact: ArtifactRecord {
                    kind: artifact.kind(),
                    version: version.as_str().to_string(),
                    store_path: layout.store_path_of(&entry_dir)?,
                },
                source: SourceRecord::local(FIRMWARE_SOURCE_KIND, pup_sha256),
                // A firmware entry's per-file manifest is the
                // `firmware.toml` inside its tree, so the record carries
                // acquisition data and nothing else.
                title: None,
                files: std::collections::BTreeMap::new(),
                rap: None,
            },
            version: version.as_str().to_string(),
            entry_dir,
            record_path,
            manifest_entries,
            omissions,
            files: tally.files,
            packages: tally.packages,
            replaced,
        };
        Ok((staged, version_lock))
    })?;

    let rename_retries = commit(&staging_root, &staged, progress)?;
    progress.finished();

    Ok(FirmwareInstallOutcome {
        manifest_path: staged.entry_dir.join(FLASH_MOUNT).join(MANIFEST_FILE),
        version: staged.version,
        entry_dir: staged.entry_dir,
        manifest_entries: staged.manifest_entries,
        omissions: staged.omissions,
        packages: staged.packages,
        files: staged.files,
        record_path: staged.record_path,
        replaced: staged.replaced,
        rename_retries,
    })
}

/// The staged install, complete and gated, awaiting its commit rename.
struct Staged {
    record: InstallRecord,
    version: String,
    entry_dir: PathBuf,
    record_path: PathBuf,
    manifest_entries: usize,
    omissions: Vec<ManifestOmission>,
    files: usize,
    packages: Vec<PackageSummary>,
    replaced: bool,
}

/// An absent record is success; every other failure is named.
fn remove_record(record_path: &Path) -> Result<(), FirmwareInstallError> {
    match std::fs::remove_file(record_path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(io_err("remove", record_path)(e)),
    }
}

/// Drop the entry's record, rename the staged tree onto it, then write
/// the new record. Returns the refusals the rename outwaited.
///
/// No rollback -- see the module invariants for the residue a fault
/// leaves and which retry clears it.
fn commit(
    staging_root: &Path,
    staged: &Staged,
    progress: &dyn ProgressSink,
) -> Result<u32, FirmwareInstallError> {
    progress.phase(FirmwarePhase::Committing.code());
    if let Some(parent) = staged.entry_dir.parent() {
        std::fs::create_dir_all(parent).map_err(io_err("create dir", parent))?;
    }
    remove_record(&staged.record_path)?;
    if std::fs::exists(&staged.entry_dir).map_err(io_err("stat", &staged.entry_dir))? {
        // Its own phase: removing a whole installed firmware is
        // genuinely slow.
        progress.phase(FirmwarePhase::Clearing.code());
        std::fs::remove_dir_all(&staged.entry_dir).map_err(io_err("remove", &staged.entry_dir))?;
        progress.phase(FirmwarePhase::Committing.code());
    }
    let rename_retries = rename_with_retry(staging_root, &staged.entry_dir).map_err(|source| {
        FirmwareInstallError::CommitFailed {
            staging_root: staging_root.to_path_buf(),
            entry_dir: staged.entry_dir.clone(),
            source,
        }
    })?;

    if let Some(parent) = staged.record_path.parent() {
        std::fs::create_dir_all(parent).map_err(io_err("create dir", parent))?;
    }
    let text = staged.record.to_toml()?;
    std::fs::write(&staged.record_path, text).map_err(io_err("write", &staged.record_path))?;
    Ok(rename_retries)
}

#[cfg(test)]
#[path = "tests/install_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/commit_discipline_tests.rs"]
mod commit_discipline_tests;

#[cfg(test)]
#[path = "tests/installed_record_tests.rs"]
mod installed_record_tests;
