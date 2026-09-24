//! What the versioned store holds, read from its install records.
//!
//! The records are the store's index: a record names every directory a
//! boot composes from. A record this build cannot read fails the whole
//! walk. An inventory that omitted an installed version would leave the
//! selection rules reporting one candidate where there are two.
//!
//! The module holds the one walk over a records directory. The
//! inventory reads through it, and so do the uninstallers' version
//! listings and the pre-store probe.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use cellgov_ps3_abi::format::dev_flash::FLASH_MOUNT;

use crate::store::layout::{
    base_record_file, firmware_record_file, firmware_record_version, update_record_file,
    update_record_version, ArtifactKind, StoreKeyError, StoreLayout, TitleId, TitleTree,
    BASE_GAME_VER, INSTALL_RECORD_SUFFIX,
};
use crate::store::pre_store::{preflight, PreStoreError};
use crate::store::record::{
    stored_kernel, CoreOsRecord, InstallRecord, InstallRecordParseError, KernelAbsence,
    KernelRecord,
};

/// Why the store's install records could not be read.
#[derive(Debug, thiserror::Error)]
pub enum InventoryError {
    /// The root still holds the layout that came before the store,
    /// or a probe could not say which layout it holds.
    #[error("{0}")]
    PreStore(#[from] PreStoreError),
    /// A records directory exists but could not be enumerated.
    #[error("reading the store's install records under {}: {source}", dir.display())]
    ReadDir {
        /// The directory that could not be enumerated.
        dir: PathBuf,
        /// The underlying failure.
        #[source]
        source: std::io::Error,
    },
    /// One directory entry under a records directory could not be read.
    #[error("reading install record {}: {source}", path.display())]
    ReadRecord {
        /// The record file.
        path: PathBuf,
        /// The underlying failure.
        #[source]
        source: std::io::Error,
    },
    /// A record this build does not read, or one that fails the record
    /// gate.
    #[error("install record {}: {source}", path.display())]
    ParseRecord {
        /// The record file.
        path: PathBuf,
        /// Why the record was refused. Boxed: the TOML parse error
        /// it wraps is far larger than every other variant.
        #[source]
        source: Box<InstallRecordParseError>,
    },
    /// A record filed under one kind's directory that declares another.
    #[error(
        "install record {} declares kind {} but sits under the {} records",
        path.display(),
        found.as_str(),
        expected.as_str()
    )]
    KindMismatch {
        /// The record file.
        path: PathBuf,
        /// The kind the directory implies.
        expected: ArtifactKind,
        /// The kind the record declares.
        found: ArtifactKind,
    },
    /// A title record whose `[title] title_id` names a different title
    /// than the directory it is filed under.
    #[error(
        "install record {} declares title_id {found:?} but sits under {expected:?}",
        path.display()
    )]
    TitleIdMismatch {
        /// The record file.
        path: PathBuf,
        /// The title directory the record is filed under.
        expected: String,
        /// The id the record declares.
        found: String,
    },
    /// A record directory whose name no store path could carry.
    #[error("install record directory {} is not a store title id: {source}", dir.display())]
    UnsafeTitleId {
        /// The directory that was walked.
        dir: PathBuf,
        /// Why the name was refused.
        #[source]
        source: StoreKeyError,
    },
    /// A record filed under a name that does not carry the version it
    /// declares.
    ///
    /// The filename is the version key. Without this refusal, the later
    /// of two records claiming one version replaces the earlier in the
    /// inventory.
    #[error(
        "install record {} declares a version the store files as {expected}; the record is \
         stale, or a second claim on one version",
        path.display()
    )]
    MisfiledRecord {
        /// The record file.
        path: PathBuf,
        /// The name the store files this record's version under.
        expected: String,
    },
    /// A license directory could not be probed.
    #[error("probing the license directory {}: {source}", dir.display())]
    ProbeExdata {
        /// The directory that could not be probed.
        dir: PathBuf,
        /// The underlying failure.
        #[source]
        source: std::io::Error,
    },
}

/// A records directory that exists but that the walk could not
/// enumerate.
#[derive(Debug, thiserror::Error)]
#[error("reading the store's install records under {}: {source}", .dir.display())]
pub(crate) struct RecordDirError {
    /// The directory the walk could not enumerate.
    pub(crate) dir: PathBuf,
    /// The underlying failure.
    #[source]
    pub(crate) source: std::io::Error,
}

impl From<RecordDirError> for InventoryError {
    fn from(error: RecordDirError) -> Self {
        Self::ReadDir {
            dir: error.dir,
            source: error.source,
        }
    }
}

/// One installed firmware version.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FirmwareEntry {
    /// The console-visible version string the entry is keyed on.
    pub version: String,
    /// The entry directory, from the record's `store_path`.
    pub entry_dir: PathBuf,
    /// SHA-256 over the PUP the entry was installed from.
    pub pup_sha256: String,
    /// The record's `[core_os]` block: the kernel stored beside
    /// `dev_flash/`, or why there is none. `None` for a record that
    /// predates the block.
    pub core_os: Option<CoreOsRecord>,
}

impl FirmwareEntry {
    /// The `dev_flash` tree the guest sees at `/dev_flash`.
    #[must_use]
    pub fn dev_flash_dir(&self) -> PathBuf {
        self.entry_dir.join(FLASH_MOUNT)
    }

    /// The kernel the entry stores, or why it stores none.
    ///
    /// # Errors
    ///
    /// [`KernelAbsence`] when the entry holds no stored kernel.
    pub fn stored_kernel(&self) -> Result<&KernelRecord, KernelAbsence<'_>> {
        stored_kernel(self.core_os.as_ref())
    }
}

/// A title's base install: the one full tree per title id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BaseEntry {
    /// The version the record names the base by: PARAM.SFO `APP_VER`,
    /// or `VERSION` when the tree's table carries no `APP_VER`. The
    /// table itself says which; see [`Self::param_sfo_path`].
    pub version: String,
    /// The tree directory, from the record's `store_path`.
    pub dir: PathBuf,
    /// Which mount the tree backs.
    pub tree: TitleTree,
    /// Install distribution tag, rendered in the selection banner.
    pub distribution: String,
    /// SHA-256 over the container the base was installed from.
    pub source_sha256: String,
    /// The tree's PARAM.SFO `PS3_SYSTEM_VER` as the record holds it;
    /// `None` when the table declared none or the record predates the
    /// field.
    pub system_ver: Option<String>,
    /// Version key of the firmware entry the disc install registered
    /// from the system software the disc shipped in `PS3_UPDATE/`.
    ///
    /// `None` when:
    ///
    /// - the base came from a PKG;
    /// - the disc carried no update package;
    /// - the install declined it (`--no-firmware`);
    /// - the record predates the field.
    pub shipped_firmware: Option<String>,
}

impl BaseEntry {
    /// The PARAM.SFO the version was read from.
    #[must_use]
    pub fn param_sfo_path(&self) -> PathBuf {
        self.tree.param_sfo_in(&self.dir)
    }
}

/// One installed update version of a title.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateEntry {
    /// The store key, verbatim from the update PKG's PARAM.SFO:
    /// `APP_VER`, or `VERSION` when the table carries no `APP_VER`.
    pub version: String,
    /// The `dev_hdd0/game` tree this update installs: the `game` child
    /// of the entry directory the record's `store_path` names.
    pub dir: PathBuf,
    /// SHA-256 over the update PKG.
    pub source_sha256: String,
    /// Lowest firmware the publishing metadata declared for the update.
    pub min_system_ver: Option<String>,
    /// The tree's PARAM.SFO `PS3_SYSTEM_VER` as the record holds it;
    /// see [`BaseEntry::system_ver`].
    pub system_ver: Option<String>,
}

impl UpdateEntry {
    /// The PARAM.SFO the version key was read from.
    #[must_use]
    pub fn param_sfo_path(&self) -> PathBuf {
        TitleTree::Game.param_sfo_in(&self.dir)
    }
}

/// Every store entry belonging to one title id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TitleEntry {
    /// The store key, and the guest directory name under
    /// `/dev_hdd0/game` or `/dev_bdvd`.
    pub title_id: String,
    /// `None` for an orphan: updates archived ahead of the base.
    pub base: Option<BaseEntry>,
    /// Installed updates, keyed and ordered by version string.
    pub updates: BTreeMap<String, UpdateEntry>,
    /// Where this title's own license files are stored, composed into
    /// the guest license directory at boot.
    pub exdata_dir: PathBuf,
}

impl TitleEntry {
    /// The versions `--game-ver` accepts, in the order a refusal lists
    /// them: [`BASE_GAME_VER`] first when a base is installed, then the
    /// update versions.
    #[must_use]
    pub fn candidates(&self) -> Vec<String> {
        let mut out = Vec::with_capacity(self.updates.len() + 1);
        if self.base.is_some() {
            out.push(BASE_GAME_VER.to_string());
        }
        out.extend(self.updates.keys().cloned());
        out
    }
}

/// The store's firmware and title entries under one VFS root.
#[derive(Debug, Clone)]
pub struct StoreInventory {
    root: PathBuf,
    live_exdata_dir: PathBuf,
    firmware: BTreeMap<String, FirmwareEntry>,
    titles: BTreeMap<String, TitleEntry>,
}

impl StoreInventory {
    /// Reads every install record under `root`.
    ///
    /// A missing records directory is an empty inventory: nothing is
    /// installed yet.
    ///
    /// # Errors
    ///
    /// Every [`InventoryError`]:
    ///
    /// - a root that still holds the pre-store layout;
    /// - an unreadable directory or file;
    /// - a record this build does not read;
    /// - a record whose declared identity disagrees with where the
    ///   store files it.
    pub fn read(root: &Path) -> Result<Self, InventoryError> {
        preflight(root)?;
        let layout = StoreLayout::new(root);
        let mut firmware = BTreeMap::new();
        for path in record_files(&layout.firmware_records_dir())? {
            let record = load_record(&path)?;
            expect_kind(&path, ArtifactKind::Firmware, record.artifact.kind)?;
            expect_record_name(&path, &firmware_record_file(&record.artifact.version))?;
            firmware.insert(
                record.artifact.version.clone(),
                FirmwareEntry {
                    version: record.artifact.version,
                    entry_dir: layout.resolve_store_path(&record.artifact.store_path),
                    pup_sha256: record.source.sha256.to_hex(),
                    core_os: record.core_os,
                },
            );
        }
        let base_record = base_record_file();
        let mut titles = BTreeMap::new();
        for title_dir in title_record_dirs(&layout.title_records_root())? {
            let title_id = file_name(&title_dir);
            let key = TitleId::new(&title_id).map_err(|source| InventoryError::UnsafeTitleId {
                dir: title_dir.clone(),
                source,
            })?;
            let mut entry = TitleEntry {
                title_id: title_id.clone(),
                base: None,
                updates: BTreeMap::new(),
                exdata_dir: layout.title_exdata_dir(&key),
            };
            for path in record_files(&title_dir)? {
                let is_base = file_name(&path) == base_record;
                let record = load_record(&path)?;
                let expected = if is_base {
                    ArtifactKind::TitleBase
                } else {
                    ArtifactKind::TitleUpdate
                };
                expect_kind(&path, expected, record.artifact.kind)?;
                let title = record.title.as_ref().ok_or_else(|| {
                    // `InstallRecord::parse` refuses a title record with
                    // no `[title]` block, so this arm means the record
                    // gate and this reader disagree.
                    InventoryError::TitleIdMismatch {
                        path: path.clone(),
                        expected: title_id.clone(),
                        found: String::new(),
                    }
                })?;
                if title.title_id != title_id {
                    return Err(InventoryError::TitleIdMismatch {
                        path,
                        expected: title_id,
                        found: title.title_id.clone(),
                    });
                }
                let entry_dir = layout.resolve_store_path(&record.artifact.store_path);
                if is_base {
                    entry.base = Some(BaseEntry {
                        version: record.artifact.version.clone(),
                        dir: entry_dir,
                        tree: title.tree(),
                        distribution: title.distribution.clone(),
                        source_sha256: record.source.sha256.to_hex(),
                        system_ver: title.system_ver.clone(),
                        shipped_firmware: title.shipped_firmware.clone(),
                    });
                } else {
                    expect_record_name(&path, &update_record_file(&record.artifact.version))?;
                    // The installer stages the patch tree under the
                    // `game/` child of the entry directory.
                    entry.updates.insert(
                        record.artifact.version.clone(),
                        UpdateEntry {
                            version: record.artifact.version.clone(),
                            dir: entry_dir.join(TitleTree::Game.dir_name()),
                            source_sha256: record.source.sha256.to_hex(),
                            min_system_ver: record.source.min_system_ver.clone(),
                            system_ver: title.system_ver.clone(),
                        },
                    );
                }
            }
            titles.insert(title_id, entry);
        }
        Ok(Self {
            root: root.to_path_buf(),
            live_exdata_dir: layout.live_exdata_dir(),
            firmware,
            titles,
        })
    }

    /// Every license directory the composed union serves, live one
    /// first, then one per store title in title-id order.
    ///
    /// A directory that does not exist is dropped: an installed title
    /// need not hold a license.
    ///
    /// # Errors
    ///
    /// [`InventoryError::ProbeExdata`] when a directory can be neither
    /// read nor shown absent. A license root dropped on an unreadable
    /// probe would boot the title on the vault's free klicensee instead
    /// of the RAP it holds.
    pub fn exdata_roots(&self) -> Result<Vec<PathBuf>, InventoryError> {
        let mut out = Vec::new();
        for dir in std::iter::once(&self.live_exdata_dir)
            .chain(self.titles.values().map(|t| &t.exdata_dir))
        {
            let present = dir_exists(dir).map_err(|source| InventoryError::ProbeExdata {
                dir: dir.clone(),
                source,
            })?;
            if present {
                out.push(dir.clone());
            }
        }
        Ok(out)
    }

    /// The VFS root the entries were read under, named in refusals.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Installed firmware versions, ascending by version string.
    #[must_use]
    pub fn firmware_versions(&self) -> Vec<String> {
        self.firmware.keys().cloned().collect()
    }

    /// Every installed firmware entry, ascending by version key.
    pub fn firmware_entries(&self) -> impl Iterator<Item = &FirmwareEntry> {
        self.firmware.values()
    }

    /// Every title with a store entry, ascending by title id.
    pub fn titles(&self) -> impl Iterator<Item = &TitleEntry> {
        self.titles.values()
    }

    /// The entry for one firmware version, if installed.
    #[must_use]
    pub fn firmware(&self, version: &str) -> Option<&FirmwareEntry> {
        self.firmware.get(version)
    }

    /// The only installed firmware, or `None` when zero or several are.
    #[must_use]
    pub fn sole_firmware(&self) -> Option<&FirmwareEntry> {
        match self.firmware.values().collect::<Vec<_>>().as_slice() {
            [only] => Some(only),
            _ => None,
        }
    }

    /// The store entries for one title id, if it has any.
    #[must_use]
    pub fn title(&self, title_id: &str) -> Option<&TitleEntry> {
        self.titles.get(title_id)
    }
}

/// The versions that name the firmware records under `layout`,
/// ascending. The listing reads names only, not the records.
pub(crate) fn firmware_record_versions(
    layout: &StoreLayout,
) -> Result<Vec<String>, RecordDirError> {
    let mut out: Vec<String> = record_files(&layout.firmware_records_dir())?
        .iter()
        .filter_map(|path| firmware_record_version(&file_name(path)).map(str::to_string))
        .collect();
    out.sort();
    Ok(out)
}

/// The versions that name `title_id`'s update records, ascending.
/// The listing reads names only, not the records.
pub(crate) fn update_record_versions(
    layout: &StoreLayout,
    title_id: &TitleId,
) -> Result<Vec<String>, RecordDirError> {
    let mut out: Vec<String> = record_files(&layout.title_records_dir(title_id))?
        .iter()
        .filter_map(|path| update_record_version(&file_name(path)).map(str::to_string))
        .collect();
    out.sort();
    Ok(out)
}

/// The `<name>.install.toml` entries directly under `dir`, sorted by
/// name so a walk does not inherit the host's enumeration order.
///
/// A missing directory yields no entries: nothing of that kind is
/// recorded.
pub(crate) fn record_files(dir: &Path) -> Result<Vec<PathBuf>, RecordDirError> {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(source) => {
            return Err(RecordDirError {
                dir: dir.to_path_buf(),
                source,
            })
        }
    };
    let mut out = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|source| RecordDirError {
            dir: dir.to_path_buf(),
            source,
        })?;
        let path = entry.path();
        if file_name(&path).ends_with(INSTALL_RECORD_SUFFIX) {
            out.push(path);
        }
    }
    out.sort();
    Ok(out)
}

/// The per-title record directories under `titles`, sorted by name.
fn title_record_dirs(titles: &Path) -> Result<Vec<PathBuf>, InventoryError> {
    let entries = match std::fs::read_dir(titles) {
        Ok(e) => e,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(source) => {
            return Err(InventoryError::ReadDir {
                dir: titles.to_path_buf(),
                source,
            })
        }
    };
    let mut out = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|source| InventoryError::ReadDir {
            dir: titles.to_path_buf(),
            source,
        })?;
        let path = entry.path();
        // Skipping an unreadable directory would drop every record of
        // that title and leave the selection rules reporting one
        // candidate.
        let is_dir = dir_exists(&path).map_err(|source| InventoryError::ReadDir {
            dir: path.clone(),
            source,
        })?;
        if is_dir {
            out.push(path);
        }
    }
    out.sort();
    Ok(out)
}

/// Whether `dir` names an existing directory.
///
/// # Errors
///
/// Every probe failure except `NotFound`, so an unreadable directory
/// never reads as an absent one.
pub fn dir_exists(dir: &Path) -> Result<bool, std::io::Error> {
    match std::fs::metadata(dir) {
        Ok(md) => Ok(md.is_dir()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e),
    }
}

fn load_record(path: &Path) -> Result<InstallRecord, InventoryError> {
    let text = std::fs::read_to_string(path).map_err(|source| InventoryError::ReadRecord {
        path: path.to_path_buf(),
        source,
    })?;
    InstallRecord::parse(&text).map_err(|source| InventoryError::ParseRecord {
        path: path.to_path_buf(),
        source: Box::new(source),
    })
}

fn expect_kind(
    path: &Path,
    expected: ArtifactKind,
    found: ArtifactKind,
) -> Result<(), InventoryError> {
    if expected == found {
        Ok(())
    } else {
        Err(InventoryError::KindMismatch {
            path: path.to_path_buf(),
            expected,
            found,
        })
    }
}

/// Refuses a record filed under a name the store would not write it
/// under; see [`InventoryError::MisfiledRecord`].
fn expect_record_name(path: &Path, expected: &str) -> Result<(), InventoryError> {
    if file_name(path) == expected {
        Ok(())
    } else {
        Err(InventoryError::MisfiledRecord {
            path: path.to_path_buf(),
            expected: expected.to_string(),
        })
    }
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

#[cfg(test)]
#[path = "tests/inventory_tests.rs"]
mod tests;
