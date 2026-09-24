//! The store keys: [`TitleId`], [`VersionKey`], and the [`Artifact`] identity built from them.

use std::path::{Path, PathBuf};

use cellgov_ps3_abi::format::param_sfo::PARAM_SFO_FILE;
use cellgov_ps3_abi::format::title_tree::DISC_GAME_DIR;
use serde::{Deserialize, Serialize};

use super::safety::is_safe_component;

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
