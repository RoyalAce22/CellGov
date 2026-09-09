//! What the versioned store holds, read from its install records.
//!
//! The records are the store's index: a record names every directory a
//! boot composes from. A record this build cannot read fails the whole
//! walk. An inventory that omitted an installed version would leave the
//! selection rules reporting one candidate where there are two.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use cellgov_install::store::{
    ArtifactKind, InstallRecord, InstallRecordParseError, PreStoreError, StoreKeyError,
    StoreLayout, TitleId, TitleTree,
};
use cellgov_ps3_abi::dev_flash::FLASH_MOUNT;
use cellgov_ps3_abi::param_sfo::PARAM_SFO_FILE;
use cellgov_ps3_abi::title_tree::DISC_GAME_DIR;

use crate::game::manifest::BASE_GAME_VER;

/// Suffix every install-record filename carries.
const INSTALL_RECORD_SUFFIX: &str = ".install.toml";

/// Record filename of a title's base entry.
const BASE_RECORD_FILE: &str = "base.install.toml";

/// Prefix an update record's filename carries before its version key.
const UPDATE_RECORD_PREFIX: &str = "update-";

/// The `[title] distribution` tag a disc dump installs under, and the
/// one value that makes a base entry a `dev_bdvd` tree.
const DISC_DISTRIBUTION: &str = "disc-iso";

/// Why the store's install records could not be read.
#[derive(Debug, thiserror::Error)]
pub(crate) enum InventoryError {
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

/// One installed firmware version.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FirmwareEntry {
    /// The console-visible version string the entry is keyed on.
    pub version: String,
    /// The entry directory, from the record's `store_path`.
    pub entry_dir: PathBuf,
    /// SHA-256 over the PUP the entry was installed from.
    pub pup_sha256: String,
}

impl FirmwareEntry {
    /// The `dev_flash` tree the guest sees at `/dev_flash`.
    pub(crate) fn dev_flash_dir(&self) -> PathBuf {
        self.entry_dir.join(FLASH_MOUNT)
    }
}

/// A title's base install: the one full tree per title id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BaseEntry {
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
}

impl BaseEntry {
    /// The PARAM.SFO the version was read from.
    pub(crate) fn param_sfo_path(&self) -> PathBuf {
        match self.tree {
            TitleTree::Disc => self.dir.join(DISC_GAME_DIR).join(PARAM_SFO_FILE),
            TitleTree::Game => self.dir.join(PARAM_SFO_FILE),
        }
    }
}

/// One installed update version of a title.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct UpdateEntry {
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
    pub(crate) fn param_sfo_path(&self) -> PathBuf {
        self.dir.join(PARAM_SFO_FILE)
    }
}

/// Every store entry belonging to one title id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TitleEntry {
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
    /// them: `base` first when a base is installed, then the update
    /// versions.
    pub(crate) fn candidates(&self) -> Vec<String> {
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
pub(crate) struct StoreInventory {
    root: PathBuf,
    live_exdata_dir: PathBuf,
    firmware: BTreeMap<String, FirmwareEntry>,
    titles: BTreeMap<String, TitleEntry>,
}

impl StoreInventory {
    /// Read every install record under `root`.
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
    pub(crate) fn read(root: &Path) -> Result<Self, InventoryError> {
        cellgov_install::store::preflight(root)?;
        let layout = StoreLayout::new(root);
        let installs = layout.installs_dir();
        let mut firmware = BTreeMap::new();
        for path in record_files(&installs.join(ArtifactKind::Firmware.as_str()))? {
            let record = load_record(&path)?;
            expect_kind(&path, ArtifactKind::Firmware, record.artifact.kind)?;
            expect_record_name(
                &path,
                &format!("{}{INSTALL_RECORD_SUFFIX}", record.artifact.version),
            )?;
            firmware.insert(
                record.artifact.version.clone(),
                FirmwareEntry {
                    version: record.artifact.version,
                    entry_dir: layout.resolve_store_path(&record.artifact.store_path),
                    pup_sha256: record.source.sha256.to_hex(),
                },
            );
        }
        let mut titles = BTreeMap::new();
        for title_dir in title_record_dirs(&installs.join("titles"))? {
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
                let is_base = file_name(&path) == BASE_RECORD_FILE;
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
                        tree: if title.distribution == DISC_DISTRIBUTION {
                            TitleTree::Disc
                        } else {
                            TitleTree::Game
                        },
                        distribution: title.distribution.clone(),
                        source_sha256: record.source.sha256.to_hex(),
                        system_ver: title.system_ver.clone(),
                    });
                } else {
                    expect_record_name(
                        &path,
                        &format!(
                            "{UPDATE_RECORD_PREFIX}{}{INSTALL_RECORD_SUFFIX}",
                            record.artifact.version
                        ),
                    )?;
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
    pub(crate) fn exdata_roots(&self) -> Result<Vec<PathBuf>, InventoryError> {
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
    pub(crate) fn root(&self) -> &Path {
        &self.root
    }

    /// Installed firmware versions, ascending by version string.
    pub(crate) fn firmware_versions(&self) -> Vec<String> {
        self.firmware.keys().cloned().collect()
    }

    /// Every installed firmware entry, ascending by version key.
    pub(crate) fn firmware_entries(&self) -> impl Iterator<Item = &FirmwareEntry> {
        self.firmware.values()
    }

    /// Every title with a store entry, ascending by title id.
    pub(crate) fn titles(&self) -> impl Iterator<Item = &TitleEntry> {
        self.titles.values()
    }

    /// The entry for one firmware version, if installed.
    pub(crate) fn firmware(&self, version: &str) -> Option<&FirmwareEntry> {
        self.firmware.get(version)
    }

    /// The only installed firmware, or `None` when zero or several are.
    pub(crate) fn sole_firmware(&self) -> Option<&FirmwareEntry> {
        match self.firmware.values().collect::<Vec<_>>().as_slice() {
            [only] => Some(only),
            _ => None,
        }
    }

    /// The store entries for one title id, if it has any.
    pub(crate) fn title(&self, title_id: &str) -> Option<&TitleEntry> {
        self.titles.get(title_id)
    }
}

/// The `<name>.install.toml` files directly under `dir`, sorted by
/// name so the walk does not inherit the host's enumeration order.
///
/// A missing directory yields no files: nothing of that kind is
/// installed.
fn record_files(dir: &Path) -> Result<Vec<PathBuf>, InventoryError> {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(source) => {
            return Err(InventoryError::ReadDir {
                dir: dir.to_path_buf(),
                source,
            })
        }
    };
    let mut out = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|source| InventoryError::ReadDir {
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
pub(crate) fn dir_exists(dir: &Path) -> Result<bool, std::io::Error> {
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

/// Refuse a record filed under a name the store would not write it
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

#[cfg(test)]
#[path = "tests/pre_store_tests.rs"]
mod pre_store_tests;
