//! The store's directory and record names, and [`StoreLayout`], which joins them under one VFS root.

use std::path::{Path, PathBuf};

use cellgov_ps3_abi::format::hdd0::{EXDATA_DIR, HDD0_MOUNT, HOME_DIR, USER_DIR};

use super::keys::{Artifact, TitleId};
use super::safety::{is_safe_component, store_path_is_safe};
use super::siblings::{FIRMWARE_STAGING_DIR, FIRMWARE_STAGING_LOCK};

/// Where the installers write, and where a reader looks for the
/// matching records, when no root is named.
pub const DEFAULT_VFS_ROOT: &str = "vfs";

/// The exdata directory under a `dev_hdd0` mount, where an installed
/// RAP lives: [`StoreLayout::live_exdata_dir`] for a store, or the
/// directory a boot reads RAPs from under its PS3 VFS root.
#[must_use]
pub fn hdd0_exdata_dir(dev_hdd0: &Path) -> PathBuf {
    dev_hdd0.join(HOME_DIR).join(USER_DIR).join(EXDATA_DIR)
}

/// Directory under a VFS root that holds what CellGov keeps about the
/// store rather than in it.
const CELLGOV_DIR: &str = ".cellgov";

/// Directory holding the firmware entries, under the root and under the
/// records and locks directories alike.
const FIRMWARE_DIR: &str = "firmware";

/// Directory holding the per-title entries, under the root and under the
/// records and locks directories alike.
const TITLES_DIR: &str = "titles";

/// Suffix every install-record filename carries.
pub(crate) const INSTALL_RECORD_SUFFIX: &str = ".install.toml";

/// Prefix an update's record and lock filenames carry before its
/// version key.
const UPDATE_RECORD_PREFIX: &str = "update-";

/// The version a title's base install answers to. `--game-ver` selects
/// the base by it, and the store files the base record under it.
pub const BASE_GAME_VER: &str = "base";

/// The record filename of firmware `version`.
pub(crate) fn firmware_record_file(version: &str) -> String {
    format!("{version}{INSTALL_RECORD_SUFFIX}")
}

/// The record filename of a title's base install.
pub(crate) fn base_record_file() -> String {
    format!("{BASE_GAME_VER}{INSTALL_RECORD_SUFFIX}")
}

/// The record filename of a title's update `version`.
pub(crate) fn update_record_file(version: &str) -> String {
    format!("{UPDATE_RECORD_PREFIX}{version}{INSTALL_RECORD_SUFFIX}")
}

/// The version a firmware record filename carries, or `None` for a
/// name that is no record.
pub(crate) fn firmware_record_version(file_name: &str) -> Option<&str> {
    file_name.strip_suffix(INSTALL_RECORD_SUFFIX)
}

/// The version an update record filename carries, or `None` for a name
/// that is no update record.
pub(crate) fn update_record_version(file_name: &str) -> Option<&str> {
    file_name
        .strip_prefix(UPDATE_RECORD_PREFIX)?
        .strip_suffix(INSTALL_RECORD_SUFFIX)
}

/// Directory inside a firmware entry, beside `dev_flash/`, that holds
/// what the install copied from the CoreOS package. No mount composes
/// it, so nothing a boot loads comes from it.
pub const CORE_OS_DIR: &str = "core_os";

/// Why a directory could not be expressed as a record `store_path`.
#[derive(Debug, thiserror::Error)]
pub enum StorePathError {
    /// The directory is not under the VFS root the record describes.
    #[error("{} is not under the VFS root {}", dir.display(), root.display())]
    OutsideRoot {
        /// The directory that was to be recorded.
        dir: PathBuf,
        /// The root it was measured against.
        root: PathBuf,
    },
    /// A path component is not valid UTF-8 and so cannot be written to
    /// a TOML record.
    #[error("{} has a path component that is not valid UTF-8", dir.display())]
    NonUtf8 {
        /// The offending directory.
        dir: PathBuf,
    },
    /// The VFS root itself was passed as an entry directory.
    #[error("the VFS root {} is not a store entry", root.display())]
    IsRoot {
        /// The root that was passed as an entry directory.
        root: PathBuf,
    },
    /// A component the record parse gate would refuse.
    #[error("{} has component {component:?}, which no store path may name", dir.display())]
    UnsafeComponent {
        /// The directory that was to be recorded.
        dir: PathBuf,
        /// The component the record gate refuses.
        component: String,
    },
}

/// A record's path relative to [`StoreLayout::installs_dir`].
///
/// Records key on (kind, id, version), so no two entries can claim one
/// record.
#[must_use]
pub fn record_rel_path(artifact: &Artifact) -> PathBuf {
    match artifact {
        Artifact::Firmware { version } => {
            Path::new(FIRMWARE_DIR).join(firmware_record_file(version.as_str()))
        }
        Artifact::TitleBase { title_id } => Path::new(TITLES_DIR)
            .join(title_id.as_str())
            .join(base_record_file()),
        Artifact::TitleUpdate { title_id, version } => Path::new(TITLES_DIR)
            .join(title_id.as_str())
            .join(update_record_file(version.as_str())),
    }
}

/// The components of `path` below `root`, or `None` when `path` is not
/// under `root` or climbs out of it. The walk skips a `.` component.
#[must_use]
pub fn components_under<'a>(root: &Path, path: &'a Path) -> Option<Vec<&'a std::ffi::OsStr>> {
    let rel = path.strip_prefix(root).ok()?;
    let mut parts = Vec::new();
    for component in rel.components() {
        match component {
            std::path::Component::Normal(part) => parts.push(part),
            std::path::Component::CurDir => {}
            _ => return None,
        }
    }
    Some(parts)
}

/// Resolves store paths under one VFS root.
#[derive(Debug, Clone)]
pub struct StoreLayout {
    root: PathBuf,
}

impl StoreLayout {
    /// Bind the resolver to a VFS root.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// Install records for this root, as `<kind>/...install.toml` files.
    ///
    /// They live inside the root they describe, so relocating the VFS
    /// carries them along.
    #[must_use]
    pub fn installs_dir(&self) -> PathBuf {
        self.root.join(CELLGOV_DIR).join("installs")
    }

    /// Advisory lock files, keyed the way records are.
    ///
    /// They live outside every directory they guard, and outside the
    /// records directory a reader enumerates.
    #[must_use]
    pub fn locks_dir(&self) -> PathBuf {
        self.root.join(CELLGOV_DIR).join("locks")
    }

    /// Where the lock for `artifact` lives.
    ///
    /// One artifact names one path, and no two artifacts name the same
    /// path. Two writers of one artifact therefore meet on one file,
    /// and two writers of different artifacts never do.
    #[must_use]
    pub fn lock_path(&self, artifact: &Artifact) -> PathBuf {
        let rel = match artifact {
            Artifact::Firmware { version } => {
                Path::new(FIRMWARE_DIR).join(format!("{}.lock", version.as_str()))
            }
            Artifact::TitleBase { title_id } => Path::new(TITLES_DIR)
                .join(title_id.as_str())
                .join(format!("{BASE_GAME_VER}.lock")),
            Artifact::TitleUpdate { title_id, version } => Path::new(TITLES_DIR)
                .join(title_id.as_str())
                .join(format!("{UPDATE_RECORD_PREFIX}{}.lock", version.as_str())),
        };
        self.locks_dir().join(rel)
    }

    /// Where the lock for [`Self::firmware_staging_dir`] lives.
    ///
    /// A firmware install cannot name its entry until the extracted
    /// tree names a version. The staging directory is therefore what
    /// one install holds against the next.
    #[must_use]
    pub fn firmware_staging_lock_path(&self) -> PathBuf {
        self.locks_dir()
            .join(FIRMWARE_DIR)
            .join(FIRMWARE_STAGING_LOCK)
    }

    /// The directory holding one record per installed firmware version.
    #[must_use]
    pub(crate) fn firmware_records_dir(&self) -> PathBuf {
        self.installs_dir().join(FIRMWARE_DIR)
    }

    /// The directory holding one record directory per title.
    #[must_use]
    pub(crate) fn title_records_root(&self) -> PathBuf {
        self.installs_dir().join(TITLES_DIR)
    }

    /// The directory holding one title's base and update records.
    #[must_use]
    pub(crate) fn title_records_dir(&self, title_id: &TitleId) -> PathBuf {
        self.title_records_root().join(title_id.as_str())
    }

    /// Root of the versioned firmware entries.
    #[must_use]
    pub fn firmware_root(&self) -> PathBuf {
        self.root.join(FIRMWARE_DIR)
    }

    /// Where a firmware install stages before it knows its version.
    ///
    /// One fixed name under [`Self::firmware_root`], so the commit is a
    /// rename within that one directory, and the next firmware install
    /// sweeps an interrupted one's residue by name.
    #[must_use]
    pub fn firmware_staging_dir(&self) -> PathBuf {
        self.firmware_root().join(FIRMWARE_STAGING_DIR)
    }

    /// Root of the versioned title entries.
    #[must_use]
    pub fn titles_root(&self) -> PathBuf {
        self.root.join(TITLES_DIR)
    }

    /// A title's entry directory, holding its base, updates, and RAPs.
    #[must_use]
    pub fn title_dir(&self, title_id: &TitleId) -> PathBuf {
        self.titles_root().join(title_id.as_str())
    }

    /// Where a title's own RAPs are stored, composed into
    /// [`Self::live_exdata_dir`] at boot.
    ///
    /// Two titles sharing one RAP each hold a copy; boot deduplicates
    /// by content.
    #[must_use]
    pub fn title_exdata_dir(&self, title_id: &TitleId) -> PathBuf {
        self.title_dir(title_id).join("exdata")
    }

    /// The guest-visible exdata directory -- mutable user state, shared
    /// across versions and never a store entry.
    #[must_use]
    pub fn live_exdata_dir(&self) -> PathBuf {
        hdd0_exdata_dir(&self.root.join(HDD0_MOUNT))
    }

    /// The directory an install commits into with one rename, and an
    /// uninstall removes whole.
    #[must_use]
    pub fn entry_dir(&self, artifact: &Artifact) -> PathBuf {
        match artifact {
            Artifact::Firmware { version } => self.firmware_root().join(version.as_str()),
            Artifact::TitleBase { title_id } => self.title_dir(title_id).join("base"),
            Artifact::TitleUpdate { title_id, version } => self
                .title_dir(title_id)
                .join("updates")
                .join(version.as_str()),
        }
    }

    /// Where the record for `artifact` lives.
    #[must_use]
    pub fn record_path(&self, artifact: &Artifact) -> PathBuf {
        self.installs_dir().join(record_rel_path(artifact))
    }

    /// Express `dir` as a record `store_path`: relative to this root,
    /// `/`-separated, so a record moves with the tree it describes.
    ///
    /// This function measures containment lexically, on the path as
    /// written:
    ///
    /// - it refuses a `dir` that reaches this root through a symlink,
    /// - it accepts a `dir` that leaves this root through a symlink.
    ///
    /// The callers build `dir` by joining onto the root, where the two
    /// agree.
    ///
    /// # Errors
    ///
    /// [`StorePathError::OutsideRoot`] when `dir` is not under this
    /// root, [`StorePathError::IsRoot`] when it is the root itself,
    /// [`StorePathError::NonUtf8`] when a component cannot be written
    /// to TOML, and [`StorePathError::UnsafeComponent`] when a
    /// component is one the record gate would refuse on the way back
    /// in.
    pub fn store_path_of(&self, dir: &Path) -> Result<String, StorePathError> {
        let components =
            components_under(&self.root, dir).ok_or_else(|| StorePathError::OutsideRoot {
                dir: dir.to_path_buf(),
                root: self.root.clone(),
            })?;
        let mut parts = Vec::new();
        for component in components {
            let part = component.to_str().ok_or_else(|| StorePathError::NonUtf8 {
                dir: dir.to_path_buf(),
            })?;
            if !is_safe_component(part) {
                return Err(StorePathError::UnsafeComponent {
                    dir: dir.to_path_buf(),
                    component: part.to_string(),
                });
            }
            parts.push(part);
        }
        if parts.is_empty() {
            return Err(StorePathError::IsRoot {
                root: self.root.clone(),
            });
        }
        Ok(parts.join("/"))
    }

    /// Resolve a record's `store_path` back to a host directory.
    ///
    /// Infallible because the parse gate already refused any
    /// `store_path` whose components could leave this root
    /// (`InstallRecordParseError::UnsafeStorePath`).
    ///
    /// # Panics
    ///
    /// Debug builds only, when `store_path` did not come through that
    /// gate.
    #[must_use]
    pub fn resolve_store_path(&self, store_path: &str) -> PathBuf {
        debug_assert!(
            store_path_is_safe(store_path),
            "resolve_store_path called with {store_path:?}, which the record gate refuses"
        );
        let mut out = self.root.clone();
        for part in store_path.split('/').filter(|p| !p.is_empty()) {
            out.push(part);
        }
        out
    }
}
