//! Record-driven firmware uninstall, the inverse of
//! [`crate::firmware_install`].
//!
//! # Invariants
//!
//! - The record steers nothing until the pass checks it against the
//!   version the caller named: the firmware kind, and a `store_path`
//!   that is that version's own entry directory.
//! - The rename of the entry directory to a hidden sibling tombstone is
//!   the atomic point. The record goes next, then the tombstone.
//! - That rename goes through [`rename_with_retry`]; a `--verify` pass
//!   just opened every module under the entry.

use std::path::{Path, PathBuf};

use crate::store::inventory::{firmware_record_versions, RecordDirError};
use crate::store::layout::{tombstone_sibling, Artifact, ArtifactKind, StoreLayout, VersionKey};
use crate::store::lock::lock_artifact;
use crate::store::record::{InstallRecord, KernelRecord};
use crate::store::rename::{rename_with_retry, RenameRefused};

/// Why a firmware uninstall failed.
#[derive(Debug, thiserror::Error)]
pub enum FirmwareUninstallError {
    /// The pre-store check refused the root.
    #[error("{0}")]
    PreStore(#[from] crate::store::pre_store::PreStoreError),
    /// Another writer holds this version's entry.
    #[error("{0}")]
    Locked(#[from] crate::store::lock::StoreLockError),
    /// The entry directory names no tombstone sibling to rename onto.
    #[error("{0}")]
    HiddenSibling(#[from] crate::store::layout::HiddenSiblingError),
    /// The version is not usable as a store directory name, so it names
    /// no entry.
    #[error("unsafe firmware version {version:?}")]
    UnsafeVersion {
        /// The offending version.
        version: String,
        /// Which rule it broke.
        #[source]
        source: crate::store::layout::StoreKeyError,
    },
    /// No install record exists for the version; nothing to uninstall.
    #[error("no firmware {version:?} is installed; installed: {installed}")]
    NoRecord {
        /// The requested version.
        version: String,
        /// The versions that are installed, comma-separated, or
        /// `<none>`.
        installed: String,
    },
    /// Reading the install record failed (for a reason other than
    /// absence).
    #[error("read install record {}: {source}", path.display())]
    RecordRead {
        /// The record path.
        path: PathBuf,
        /// Underlying error.
        #[source]
        source: std::io::Error,
    },
    /// Enumerating the firmware records failed.
    #[error("reading the install records under {}: {source}", dir.display())]
    RecordsReadDir {
        /// The directory that could not be enumerated.
        dir: PathBuf,
        /// Underlying error.
        #[source]
        source: std::io::Error,
    },
    /// Loading the install record failed: bad TOML, or a schema this
    /// build does not read.
    #[error("parse install record: {0}")]
    RecordParse(#[from] crate::store::record::InstallRecordParseError),
    /// The record filed under this version describes some other kind of
    /// store entry, so nothing here names a firmware tree to remove.
    #[error("install record for firmware {version:?} describes a {} entry", found.as_str())]
    RecordKindMismatch {
        /// The requested version.
        version: String,
        /// The kind the record declared.
        found: ArtifactKind,
    },
    /// The record's `store_path` names a tree that is not this
    /// version's entry.
    #[error(
        "install record for firmware {version:?} names the tree {store_path:?}, which is not \
         this version's entry"
    )]
    RecordTreeForeign {
        /// The requested version.
        version: String,
        /// The `store_path` the record declared.
        store_path: String,
    },
    /// The tombstone rename stayed refused through every attempt it
    /// got.
    #[error("rename {}: {source}", path.display())]
    Rename {
        /// The entry the rename could not move to its tombstone.
        path: PathBuf,
        /// The refusal and its attempt count.
        #[source]
        source: RenameRefused,
    },
    /// A filesystem operation failed.
    #[error("{op} {}: {source}", path.display())]
    Io {
        /// The operation that failed.
        op: &'static str,
        /// The path involved.
        path: PathBuf,
        /// Underlying error.
        #[source]
        source: std::io::Error,
    },
}

/// What a firmware uninstall will remove, resolved from the record
/// before anything is touched.
///
/// [`execute`] takes `entry_dir` straight to a `remove_dir_all` and
/// repeats none of the identity checks [`plan`] ran to resolve it.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct FirmwareUninstallPlan {
    /// The version the caller named.
    pub version: String,
    /// The entry directory removed whole.
    pub entry_dir: PathBuf,
    /// The record removed with it.
    pub record_path: PathBuf,
    /// SHA-256 over the PUP the entry was installed from, so a report
    /// can name the source to reinstall.
    pub pup_sha256: String,
    /// The kernel the record says the entry stores, for a verifying
    /// removal to hash with the modules.
    pub kernel: Option<KernelRecord>,
    /// The identity the removal claims, resolved by the same gate that
    /// resolved `entry_dir`.
    artifact: Artifact,
    /// The root [`plan`] resolved against, so the removal claims its
    /// lock under the same store.
    layout: StoreLayout,
}

/// What an [`uninstall`] removed.
#[derive(Debug, Clone)]
pub struct FirmwareUninstallOutcome {
    /// The uninstalled version.
    pub version: String,
    /// The entry directory that was removed (or that was already gone).
    pub entry_removed: PathBuf,
    /// The install record that was removed.
    pub record_removed: PathBuf,
    /// Refusals the tombstone rename outwaited before it landed.
    /// Non-zero means a handle was open under the entry, typically an
    /// on-access scanner's.
    pub rename_retries: u32,
}

fn io_err<'a>(
    op: &'static str,
    path: &'a Path,
) -> impl Fn(std::io::Error) -> FirmwareUninstallError + 'a {
    move |source| FirmwareUninstallError::Io {
        op,
        path: path.to_path_buf(),
        source,
    }
}

/// The firmware versions with a record under `output_dir`, ascending.
///
/// The listing omits a record filename that is no usable
/// [`VersionKey`]: it names no entry [`plan`] could reach.
///
/// # Errors
///
/// [`FirmwareUninstallError::RecordsReadDir`] when the records
/// directory exists and cannot be enumerated. A missing directory is an
/// empty list: nothing is installed yet.
pub fn installed_versions(output_dir: &Path) -> Result<Vec<String>, FirmwareUninstallError> {
    let mut versions = firmware_record_versions(&StoreLayout::new(output_dir)).map_err(
        |RecordDirError { dir, source }| FirmwareUninstallError::RecordsReadDir { dir, source },
    )?;
    // The walk reads names lossily, so a name this host cannot decode
    // carries U+FFFD, and a bare `.install.toml` strips to nothing.
    // `VersionKey::new` accepts neither, so neither names an entry
    // `plan` could reach.
    versions.retain(|version| VersionKey::new(version).is_ok());
    Ok(versions)
}

/// Resolve what an uninstall of firmware `version` would remove.
///
/// Reads the record only and touches nothing on disk.
///
/// # Errors
///
/// - [`FirmwareUninstallError::PreStore`] when the root still holds the
///   pre-store layout.
/// - [`FirmwareUninstallError::NoRecord`] when the version is not
///   installed.
/// - The record read, parse, and identity refusals.
pub fn plan(
    version: &str,
    output_dir: &Path,
) -> Result<FirmwareUninstallPlan, FirmwareUninstallError> {
    crate::store::pre_store::preflight(output_dir)?;
    let key = VersionKey::new(version).map_err(|source| FirmwareUninstallError::UnsafeVersion {
        version: version.to_string(),
        source,
    })?;
    let layout = StoreLayout::new(output_dir);
    let artifact = Artifact::Firmware {
        version: key.clone(),
    };
    let record_path = layout.record_path(&artifact);
    let text = match std::fs::read_to_string(&record_path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let installed = installed_versions(output_dir)?;
            return Err(FirmwareUninstallError::NoRecord {
                version: version.to_string(),
                installed: if installed.is_empty() {
                    "<none>".to_string()
                } else {
                    installed.join(", ")
                },
            });
        }
        Err(source) => {
            return Err(FirmwareUninstallError::RecordRead {
                path: record_path,
                source,
            })
        }
    };
    let record = InstallRecord::parse(&text)?;
    check_record_describes(version, &layout, &artifact, &record)?;
    Ok(FirmwareUninstallPlan {
        version: version.to_string(),
        entry_dir: layout.resolve_store_path(&record.artifact.store_path),
        record_path,
        pup_sha256: record.source.sha256.to_hex(),
        kernel: record.core_os.and_then(|block| block.kernel),
        artifact,
        layout,
    })
}

/// Refuse a record that does not describe the entry the caller named.
///
/// The parse gate proves `store_path` stays under the VFS root. It
/// does not prove which entry the path names. That path aims both the
/// tombstone rename and the `remove_dir_all`.
fn check_record_describes(
    version: &str,
    layout: &StoreLayout,
    artifact: &Artifact,
    record: &InstallRecord,
) -> Result<(), FirmwareUninstallError> {
    if record.artifact.kind != ArtifactKind::Firmware {
        return Err(FirmwareUninstallError::RecordKindMismatch {
            version: version.to_string(),
            found: record.artifact.kind,
        });
    }
    let named = layout.resolve_store_path(&record.artifact.store_path);
    if named != layout.entry_dir(artifact) {
        return Err(FirmwareUninstallError::RecordTreeForeign {
            version: version.to_string(),
            store_path: record.artifact.store_path.clone(),
        });
    }
    Ok(())
}

/// Remove installed firmware `version`.
///
/// See the module invariants for the rename-then-teardown ordering. An
/// entry whose tree is already gone still removes its record and
/// succeeds. A version with no record is
/// [`FirmwareUninstallError::NoRecord`].
///
/// # Errors
///
/// Every [`plan`] refusal, plus every [`execute`] refusal.
pub fn uninstall(
    version: &str,
    output_dir: &Path,
) -> Result<FirmwareUninstallOutcome, FirmwareUninstallError> {
    execute(&plan(version, output_dir)?)
}

/// Run a plan [`plan`] already resolved.
///
/// # Errors
///
/// - The filesystem failures.
/// - [`FirmwareUninstallError::HiddenSibling`] for an entry directory
///   with no tombstone sibling.
/// - [`FirmwareUninstallError::Locked`] when another process holds this
///   version's entry. The claim precedes the first rename, so a refused
///   pass removes nothing.
pub fn execute(
    plan: &FirmwareUninstallPlan,
) -> Result<FirmwareUninstallOutcome, FirmwareUninstallError> {
    // An install of this version must not commit into the entry this
    // pass removes.
    let _lock = lock_artifact(&plan.layout, &plan.artifact)?;
    let tombstone = tombstone_sibling(&plan.entry_dir)?;
    remove_dir_if_present(&tombstone)?;

    // `Path::exists` answers false both for an absent tree and for a
    // stat that failed. A failed stat would drop the rename, then
    // remove the record, and leave the tree with nothing naming it.
    let mut rename_retries = 0u32;
    if std::fs::exists(&plan.entry_dir).map_err(io_err("stat", &plan.entry_dir))? {
        rename_retries = rename_with_retry(&plan.entry_dir, &tombstone).map_err(|source| {
            FirmwareUninstallError::Rename {
                path: plan.entry_dir.clone(),
                source,
            }
        })?;
    }

    match std::fs::remove_file(&plan.record_path) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(io_err("remove", &plan.record_path)(e)),
    }

    remove_dir_if_present(&tombstone)?;

    Ok(FirmwareUninstallOutcome {
        version: plan.version.clone(),
        entry_removed: plan.entry_dir.clone(),
        record_removed: plan.record_path.clone(),
        rename_retries,
    })
}

fn remove_dir_if_present(path: &Path) -> Result<(), FirmwareUninstallError> {
    match std::fs::remove_dir_all(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(io_err("remove", path)(e)),
    }
}

#[cfg(test)]
#[path = "tests/firmware_uninstall_tests.rs"]
mod tests;
