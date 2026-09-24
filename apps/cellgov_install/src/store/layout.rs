//! Path arithmetic for the versioned content store.
//!
//! Every store path is derived here from an [`Artifact`] identity, so a
//! writer and a reader cannot land on different directories for one
//! (kind, id, version).
//!
//! # Invariants
//!
//! - Every path a [`StoreLayout`] returns is under its root: ids and
//!   version keys are validated single path components ([`TitleId`],
//!   [`VersionKey`]).
//! - Staging and tombstone directories are hidden siblings of the
//!   directory they stand in for, so the commit and teardown renames
//!   stay inside one directory, and so on one filesystem.
//! - A lock path is never under the directory it guards. A writer
//!   renames or removes both the staging and the entry directory while
//!   it holds their lock.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use cellgov_ps3_abi::format::hdd0::{EXDATA_DIR, HDD0_MOUNT, HOME_DIR, USER_DIR};
use cellgov_ps3_abi::format::param_sfo::PARAM_SFO_FILE;
use cellgov_ps3_abi::format::title_tree::DISC_GAME_DIR;
use serde::{Deserialize, Serialize};

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

/// Whether a string is safe to use as a single path component under a
/// store root: non-empty, no leading or trailing dot, no Win32 device
/// name, and `[A-Za-z0-9._-]` only.
///
/// A leading dot collides with the `.staging-*` / `.uninstalling-*`
/// residue sharing the directory. A trailing dot is dropped when Win32
/// normalizes a path component, so `4.91.` and `4.91` would be two keys
/// naming one directory.
///
/// Win32 resolves a reserved device name to a character device. A
/// firmware version keyed `NUL` writes its record to `NUL.install.toml`,
/// which the null device discards and reads back empty.
pub(crate) fn is_safe_component(s: &str) -> bool {
    !s.is_empty()
        && !s.starts_with('.')
        && !s.ends_with('.')
        && !is_reserved_device_name(s)
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}

/// Whether `s` resolves to a Win32 character device.
///
/// The match is case-insensitive and covers the part before the first
/// dot. Every host refuses the name, so a record one platform writes is
/// a record the other reads.
fn is_reserved_device_name(s: &str) -> bool {
    let stem = s.split('.').next().unwrap_or_default();
    if ["CON", "PRN", "AUX", "NUL"]
        .iter()
        .any(|name| stem.eq_ignore_ascii_case(name))
    {
        return true;
    }
    // Ports number from 1: `COM0` and `LPT0` name no device.
    match stem.as_bytes() {
        [a, b, c, d] if d.is_ascii_digit() && *d != b'0' => {
            let head = [
                a.to_ascii_uppercase(),
                b.to_ascii_uppercase(),
                c.to_ascii_uppercase(),
            ];
            head == *b"COM" || head == *b"LPT"
        }
        _ => false,
    }
}

/// Whether a record `store_path` stays under the root it is resolved
/// against: non-empty, relative, `/`-separated, every component an
/// [`is_safe_component`] name.
///
/// Shared by [`StoreLayout::store_path_of`] and the record parse gate,
/// so the writer and the reader cannot disagree about which paths are
/// expressible.
pub(crate) fn store_path_is_safe(path: &str) -> bool {
    !path.is_empty() && path.split('/').all(is_safe_component)
}

/// Why a store key could not be used as a directory name.
#[derive(Debug, thiserror::Error)]
pub enum StoreKeyError {
    /// A title id that would not resolve to a fresh child of the titles
    /// root.
    #[error("title id {id:?} is not usable as a store directory name")]
    UnsafeTitleId {
        /// The offending id.
        id: String,
    },
    /// A version key that would not resolve to a fresh child of its
    /// entry root.
    #[error("version key {version:?} is not usable as a store directory name")]
    UnsafeVersion {
        /// The offending key.
        version: String,
    },
}

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

/// A directory that names no entry, so no hidden sibling can stand
/// beside it:
///
/// - the empty path, `.`, or `..`,
/// - a path that ends in `.` or `..`,
/// - a filesystem root,
/// - a bare Win32 drive or UNC prefix.
///
/// The sibling is a rename target and a `remove_dir_all` argument, so a
/// name derived from no entry would act on the process working
/// directory.
#[derive(Debug, thiserror::Error)]
#[error("{} has no final component, so it names no staging or tombstone sibling", dir.display())]
pub struct HiddenSiblingError {
    /// The directory the sibling was to stand beside.
    pub dir: PathBuf,
}

/// A title id used as a store directory name.
///
/// [`Self::new`] does not check the PARAM.SFO `TITLE_ID` shape of nine
/// `[A-Z]` and digit characters.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TitleId(String);

impl TitleId {
    /// Validate an id for use as a store directory name.
    ///
    /// # Errors
    ///
    /// [`StoreKeyError::UnsafeTitleId`] unless the id is usable as a
    /// single store path component.
    pub fn new(id: &str) -> Result<Self, StoreKeyError> {
        if is_safe_component(id) {
            Ok(Self(id.to_string()))
        } else {
            Err(StoreKeyError::UnsafeTitleId { id: id.to_string() })
        }
    }

    /// The id as written on disk.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A version key: Sony's version string verbatim (`4.91`, `02.51`).
///
/// Never normalized, and compared by string equality only: `02.51` and
/// `2.51` are different versions.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct VersionKey(String);

impl VersionKey {
    /// Validate a version string for use as a store directory name.
    ///
    /// # Errors
    ///
    /// [`StoreKeyError::UnsafeVersion`] unless the string is usable as
    /// a single store path component.
    pub fn new(version: &str) -> Result<Self, StoreKeyError> {
        if is_safe_component(version) {
            Ok(Self(version.to_string()))
        } else {
            Err(StoreKeyError::UnsafeVersion {
                version: version.to_string(),
            })
        }
    }

    /// The key as written on disk.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// What a store entry holds, as written to a record's `[artifact] kind`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ArtifactKind {
    /// An extracted firmware version.
    Firmware,
    /// A title's base install, from a retail PKG or disc image.
    TitleBase,
    /// One update version of a title, from an update PKG.
    TitleUpdate,
}

impl ArtifactKind {
    /// The kind as a record writes it.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Firmware => "firmware",
            Self::TitleBase => "title-base",
            Self::TitleUpdate => "title-update",
        }
    }
}

/// Which mount tree a title entry holds.
///
/// A base holds exactly one: [`TitleTree::Disc`] for a disc dump,
/// [`TitleTree::Game`] for an HDD title. An update always holds
/// [`TitleTree::Game`] -- an update PKG patches the `dev_hdd0/game`
/// tree even for a disc title.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TitleTree {
    /// The `dev_bdvd` tree (`PS3_GAME/`, `PS3_DISC.SFB`).
    Disc,
    /// The `dev_hdd0/game/<id>` tree.
    Game,
}

impl TitleTree {
    /// Subdirectory of a title entry holding this tree.
    #[must_use]
    pub fn dir_name(self) -> &'static str {
        match self {
            Self::Disc => "disc",
            Self::Game => "game",
        }
    }

    /// Where this tree keeps its PARAM.SFO, as path components below
    /// the tree root: under `PS3_GAME/` on a disc, at the root of an
    /// HDD tree.
    #[must_use]
    pub fn param_sfo_components(self) -> &'static [&'static str] {
        match self {
            Self::Disc => &[DISC_GAME_DIR, PARAM_SFO_FILE],
            Self::Game => &[PARAM_SFO_FILE],
        }
    }

    /// The PARAM.SFO's path below the tree root, `/`-separated, as an
    /// install record's `[files]` keys it.
    #[must_use]
    pub fn param_sfo_rel(self) -> String {
        self.param_sfo_components().join("/")
    }

    /// The PARAM.SFO inside the tree rooted at `tree_dir`.
    #[must_use]
    pub fn param_sfo_in(self, tree_dir: &Path) -> PathBuf {
        let mut path = tree_dir.to_path_buf();
        path.extend(self.param_sfo_components());
        path
    }
}

/// One store entry's identity: what it is, which title it belongs to,
/// and -- where the path encodes one -- which version.
///
/// A base carries no version key: there is exactly one base per title
/// id, and its install record carries the version its PARAM.SFO names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Artifact {
    /// An installed firmware, `firmware/<version>/`.
    Firmware {
        /// Version from the extracted `vsh/etc/version.txt` (`4.91`).
        version: VersionKey,
    },
    /// A title's base install, `titles/<id>/base/`.
    TitleBase {
        /// The title this base belongs to.
        title_id: TitleId,
    },
    /// One update version, `titles/<id>/updates/<version>/`.
    TitleUpdate {
        /// The title this update patches.
        title_id: TitleId,
        /// Version the update PKG's PARAM.SFO names
        /// ([`ParamSfo::named_version`], `02.51`).
        ///
        /// [`ParamSfo::named_version`]: crate::param_sfo::ParamSfo::named_version
        version: VersionKey,
    },
}

impl Artifact {
    /// The kind a record for this artifact declares.
    #[must_use]
    pub fn kind(&self) -> ArtifactKind {
        match self {
            Self::Firmware { .. } => ArtifactKind::Firmware,
            Self::TitleBase { .. } => ArtifactKind::TitleBase,
            Self::TitleUpdate { .. } => ArtifactKind::TitleUpdate,
        }
    }
}

/// The staging sibling of `final_dir`: `<parent>/.staging-<name>`.
///
/// The name is a function of the target, so an install interrupted
/// mid-stage leaves residue the next install of that target sweeps by
/// name.
///
/// # Errors
///
/// [`HiddenSiblingError`] when `final_dir` names no entry.
pub fn staging_sibling(final_dir: &Path) -> Result<PathBuf, HiddenSiblingError> {
    hidden_sibling(final_dir, "staging")
}

/// Directory a firmware install stages into, under the firmware root.
///
/// Outside [`hidden_sibling`]'s `.staging-<name>` / `.uninstalling-<name>`
/// namespace, so it cannot alias the residue of any one version:
/// `.staging-fw` would be [`staging_sibling`] of a firmware entry keyed
/// `fw`, which [`VersionKey`] accepts.
const FIRMWARE_STAGING_DIR: &str = ".firmware-staging";

/// Lock file for the staging directory every firmware install shares.
///
/// [`VersionKey`] rejects a leading dot, so no installed version claims
/// this name.
const FIRMWARE_STAGING_LOCK: &str = ".staging.lock";

/// The tombstone sibling of `final_dir`: `<parent>/.uninstalling-<name>`.
///
/// # Errors
///
/// [`HiddenSiblingError`] when `final_dir` names no entry.
pub fn tombstone_sibling(final_dir: &Path) -> Result<PathBuf, HiddenSiblingError> {
    hidden_sibling(final_dir, "uninstalling")
}

/// `<parent>/.<prefix>-<final component>`.
///
/// The sibling name carries the entry name verbatim, so two names that
/// differ only outside UTF-8 get two siblings.
fn hidden_sibling(final_dir: &Path, prefix: &str) -> Result<PathBuf, HiddenSiblingError> {
    match (final_dir.parent(), final_dir.file_name()) {
        (Some(parent), Some(name)) => {
            let mut sibling = OsString::from(format!(".{prefix}-"));
            sibling.push(name);
            Ok(parent.join(sibling))
        }
        _ => Err(HiddenSiblingError {
            dir: final_dir.to_path_buf(),
        }),
    }
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

#[cfg(test)]
#[path = "tests/layout_tests.rs"]
mod tests;
