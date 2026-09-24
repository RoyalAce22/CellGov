//! The selection contract: one candidate selects, several refuse.
//!
//! A firmware version and a title's game version resolve the same way:
//!
//! - a version the caller names must exist;
//! - with no name and exactly one candidate, that candidate selects;
//! - with no name and zero or several candidates, the store refuses and
//!   lists what is installed.
//!
//! A disc title whose record names its shipped firmware has one
//! candidate before any count: that version. When the store does not
//! hold that version, the selection refuses by name and takes no count.
//!
//! No selection accepts `latest`.
//!
//! The refusals are typed and name no command-line flag; a caller that
//! takes the version from a flag words the refusal around it.

use crate::store::inventory::{dir_exists, FirmwareEntry, StoreInventory, TitleEntry};
use crate::store::layout::BASE_GAME_VER;

/// A store firmware entry and what selected it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManagedFirmware {
    /// The selected entry.
    pub entry: FirmwareEntry,
    /// What selected this entry.
    pub selected_by: FirmwareSelectedBy,
}

/// What resolved a selection to one firmware entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FirmwareSelectedBy {
    /// The caller named it.
    Named,
    /// The title's record names it as the firmware its disc shipped.
    Shipped,
    /// It is the only firmware installed.
    Sole,
}

/// Which of a title's installed versions a boot composes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GameVersion {
    /// The base install alone.
    Base,
    /// One update version, over the base.
    Update(String),
}

impl std::fmt::Display for GameVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Base => f.write_str(BASE_GAME_VER),
            Self::Update(v) => write!(f, "update {v}"),
        }
    }
}

/// Why a firmware selection did not resolve to one installed entry.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum FirmwareSelectError {
    /// The named version is not in the store.
    #[error(
        "firmware {asked:?} is not installed under {root}; installed: {}",
        render_list(installed)
    )]
    NotInstalled {
        /// The version the caller named.
        asked: String,
        /// The VFS root the store was read under.
        root: String,
        /// Every installed version.
        installed: Vec<String>,
    },
    /// The store holds no firmware, and no record names one the title
    /// shipped with.
    #[error(
        "no firmware is installed under {root}, and no record names one this title shipped with"
    )]
    NoneInstalled {
        /// The VFS root the store was read under.
        root: String,
    },
    /// The title's record names the firmware its disc shipped, and the
    /// store no longer holds that version.
    #[error(
        "firmware {version} shipped with this disc and is recorded on its title, but is not \
         installed under {root}; installed: {}",
        render_list(installed)
    )]
    ShippedNotInstalled {
        /// The version the title's record names.
        version: String,
        /// The VFS root the store was read under.
        root: String,
        /// Every installed version.
        installed: Vec<String>,
    },
    /// Several firmwares are installed and nothing named one.
    #[error(
        "{} firmware versions are installed under {root} ({})",
        installed.len(),
        render_list(installed)
    )]
    Ambiguous {
        /// The VFS root the store was read under.
        root: String,
        /// Every installed version.
        installed: Vec<String>,
    },
    /// A record names an entry directory that is gone.
    #[error("firmware {version} is recorded under {root} but its tree at {dir} is missing")]
    TreeMissing {
        /// The version whose record the selection read.
        version: String,
        /// The VFS root the store was read under.
        root: String,
        /// The directory the record named.
        dir: String,
    },
    /// A record names an entry directory the probe could neither read
    /// nor show absent.
    #[error(
        "firmware {version} is recorded under {root} but its tree at {dir} could not be \
         probed: {reason}"
    )]
    TreeUnreadable {
        /// The version whose record the selection read.
        version: String,
        /// The VFS root the store was read under.
        root: String,
        /// The directory the record named.
        dir: String,
        /// Why the probe failed.
        reason: String,
    },
}

/// Why a game-version selection did not resolve to one installed
/// version.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum GameVersionSelectError {
    /// The named version is not installed for this title.
    #[error(
        "version {asked:?} is not installed for {title_id}; installed: {}",
        render_list(installed)
    )]
    NotInstalled {
        /// The version the caller named.
        asked: String,
        /// The title the selection searched.
        title_id: String,
        /// Every version this title has installed.
        installed: Vec<String>,
    },
    /// Several versions are installed and nothing named one.
    #[error(
        "{title_id} has {} versions installed ({})",
        installed.len(),
        render_list(installed)
    )]
    Ambiguous {
        /// The title the selection searched.
        title_id: String,
        /// Every version this title has installed.
        installed: Vec<String>,
    },
    /// Updates are archived for a title whose base is not installed.
    #[error(
        "{title_id} has update(s) {} installed but no base; an update tree patches a base \
         and cannot be composed alone",
        render_list(updates)
    )]
    OrphanUpdates {
        /// The title the selection searched.
        title_id: String,
        /// The archived update versions.
        updates: Vec<String>,
    },
}

/// A version list as a refusal renders it, or `(none)` when empty.
#[must_use]
pub fn render_list(versions: &[String]) -> String {
    if versions.is_empty() {
        "(none)".to_string()
    } else {
        versions.join(", ")
    }
}

/// Resolves a firmware selection against the store.
///
/// `asked` is the version the caller named. `shipped` is the firmware
/// version the title's record names, when it names one. With no name,
/// the shipped version is the one candidate, whatever else the store
/// holds.
///
/// # Errors
///
/// Every [`FirmwareSelectError`]:
///
/// - a named version that is not installed;
/// - a shipped version that is not installed;
/// - zero or several candidates with no name and no shipped version;
/// - a record whose tree is gone or cannot be probed.
pub fn select_firmware(
    inventory: &StoreInventory,
    asked: Option<&str>,
    shipped: Option<&str>,
) -> Result<ManagedFirmware, FirmwareSelectError> {
    let root = inventory.root().display().to_string();
    let (entry, selected_by) = match (asked, shipped) {
        (Some(version), _) => {
            let entry =
                inventory
                    .firmware(version)
                    .ok_or_else(|| FirmwareSelectError::NotInstalled {
                        asked: version.to_string(),
                        root: root.clone(),
                        installed: inventory.firmware_versions(),
                    })?;
            (entry, FirmwareSelectedBy::Named)
        }
        (None, Some(version)) => {
            let entry = inventory.firmware(version).ok_or_else(|| {
                FirmwareSelectError::ShippedNotInstalled {
                    version: version.to_string(),
                    root: root.clone(),
                    installed: inventory.firmware_versions(),
                }
            })?;
            (entry, FirmwareSelectedBy::Shipped)
        }
        (None, None) => {
            let entry = inventory.sole_firmware().ok_or_else(|| {
                let installed = inventory.firmware_versions();
                if installed.is_empty() {
                    FirmwareSelectError::NoneInstalled { root: root.clone() }
                } else {
                    FirmwareSelectError::Ambiguous {
                        root: root.clone(),
                        installed,
                    }
                }
            })?;
            (entry, FirmwareSelectedBy::Sole)
        }
    };
    let dev_flash = entry.dev_flash_dir();
    match dir_exists(&dev_flash) {
        Ok(true) => Ok(ManagedFirmware {
            entry: entry.clone(),
            selected_by,
        }),
        Ok(false) => Err(FirmwareSelectError::TreeMissing {
            version: entry.version.clone(),
            root,
            dir: dev_flash.display().to_string(),
        }),
        Err(e) => Err(FirmwareSelectError::TreeUnreadable {
            version: entry.version.clone(),
            root,
            dir: dev_flash.display().to_string(),
            reason: e.to_string(),
        }),
    }
}

/// Resolves a game-version selection against one title's store
/// entries. `asked` is the version the caller named, [`BASE_GAME_VER`]
/// for the base.
///
/// # Errors
///
/// Every [`GameVersionSelectError`].
pub fn select_game_version(
    entry: &TitleEntry,
    asked: Option<&str>,
) -> Result<GameVersion, GameVersionSelectError> {
    let candidates = entry.candidates();
    if entry.base.is_none() {
        return Err(GameVersionSelectError::OrphanUpdates {
            title_id: entry.title_id.clone(),
            updates: entry.updates.keys().cloned().collect(),
        });
    }
    match asked {
        Some(BASE_GAME_VER) => Ok(GameVersion::Base),
        Some(version) => {
            if entry.updates.contains_key(version) {
                Ok(GameVersion::Update(version.to_string()))
            } else {
                Err(GameVersionSelectError::NotInstalled {
                    asked: version.to_string(),
                    title_id: entry.title_id.clone(),
                    installed: candidates,
                })
            }
        }
        None => match candidates.as_slice() {
            [_only] => Ok(GameVersion::Base),
            _ => Err(GameVersionSelectError::Ambiguous {
                title_id: entry.title_id.clone(),
                installed: candidates,
            }),
        },
    }
}

#[cfg(test)]
#[path = "tests/select_tests.rs"]
mod tests;
