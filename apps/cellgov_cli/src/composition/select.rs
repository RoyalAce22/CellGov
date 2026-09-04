//! The selection contract: one candidate selects, several refuse.
//!
//! `--fw` and `--game-ver` resolve the same way:
//!
//! - a flag names a version that must exist;
//! - with no flag and exactly one candidate, that candidate selects;
//! - with no flag and zero or several candidates, the store refuses and
//!   lists what is installed.
//!
//! Neither flag accepts `latest`.

use std::path::PathBuf;

use super::inventory::{dir_exists, FirmwareEntry, StoreInventory, TitleEntry};

/// The `--game-ver` value naming a title's base install.
pub(crate) const BASE_VERSION: &str = "base";

/// What a boot answers `/dev_flash` from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum FirmwareChoice {
    /// A store entry, selected by `--fw` or by being the only one.
    Managed(FirmwareEntry),
    /// A raw tree named by `--firmware-dir`, outside the store. The
    /// run carries no firmware version, so nothing downstream can key
    /// on one.
    Unmanaged {
        /// The tree the flag named.
        dir: PathBuf,
    },
    /// No firmware at all: every import answers through the
    /// unresolved-import trampoline.
    None,
}

impl FirmwareChoice {
    /// The version key, or `None` for a run with no managed firmware.
    pub(crate) fn version(&self) -> Option<&str> {
        match self {
            Self::Managed(entry) => Some(entry.version.as_str()),
            Self::Unmanaged { .. } | Self::None => None,
        }
    }
}

/// Which of a title's installed versions a boot composes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum GameVersion {
    /// The base install alone.
    Base,
    /// One update version, over the base.
    Update(String),
}

impl std::fmt::Display for GameVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Base => f.write_str(BASE_VERSION),
            Self::Update(v) => write!(f, "update {v}"),
        }
    }
}

/// Why `--fw` could not resolve to one installed firmware.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub(crate) enum FirmwareSelectError {
    /// The named version is not in the store.
    #[error(
        "--fw {asked:?} is not installed under {root}; installed: {}",
        render_list(installed)
    )]
    NotInstalled {
        /// The version the flag named.
        asked: String,
        /// The VFS root the store was read under.
        root: String,
        /// Every installed version.
        installed: Vec<String>,
    },
    /// The store holds no firmware.
    #[error(
        "no firmware is installed under {root}; install one with \
         `cellgov firmware install <PS3UPDAT.PUP>`, name a tree with --firmware-dir, or set \
         {disable_env}=1 to boot with no firmware at all (every import then answers through \
         the unresolved-import trampoline)"
    )]
    NoneInstalled {
        /// The VFS root the store was read under.
        root: String,
        /// The variable that asks for a firmware-free boot.
        disable_env: &'static str,
    },
    /// Several firmwares are installed and nothing named one.
    #[error(
        "{} firmware versions are installed under {root} ({}); name the one to boot against \
         with --fw",
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
    #[error(
        "firmware {version} is recorded under {root} but its tree at {dir} is missing; \
         reinstall it, or name a tree with --firmware-dir"
    )]
    TreeMissing {
        /// The version whose record was read.
        version: String,
        /// The VFS root the store was read under.
        root: String,
        /// The directory the record named.
        dir: String,
    },
    /// A record names an entry directory that could be neither read
    /// nor shown absent.
    #[error(
        "firmware {version} is recorded under {root} but its tree at {dir} could not be \
         probed: {reason}"
    )]
    TreeUnreadable {
        /// The version whose record was read.
        version: String,
        /// The VFS root the store was read under.
        root: String,
        /// The directory the record named.
        dir: String,
        /// Why the probe failed.
        reason: String,
    },
}

/// Why `--game-ver` could not resolve to one installed version.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub(crate) enum GameVersionSelectError {
    /// The named version is not installed for this title.
    #[error(
        "--game-ver {asked:?} is not installed for {title_id}; installed: {}",
        render_list(installed)
    )]
    NotInstalled {
        /// The version the flag named.
        asked: String,
        /// The title the store was searched for.
        title_id: String,
        /// Every version this title has installed.
        installed: Vec<String>,
    },
    /// Several versions are installed and nothing named one.
    #[error(
        "{title_id} has {} versions installed ({}); name the one to boot with --game-ver",
        installed.len(),
        render_list(installed)
    )]
    Ambiguous {
        /// The title the store was searched for.
        title_id: String,
        /// Every version this title has installed.
        installed: Vec<String>,
    },
    /// Updates are archived for a title whose base is not installed.
    #[error(
        "{title_id} has update(s) {} installed but no base; an update tree patches a base \
         and cannot be composed alone. Install the base with \
         `cellgov title install <PKG|ISO>`",
        render_list(updates)
    )]
    OrphanUpdates {
        /// The title the store was searched for.
        title_id: String,
        /// The archived update versions.
        updates: Vec<String>,
    },
    /// `--game-ver` was passed for a title that ships inside the
    /// firmware.
    #[error(
        "--game-ver does not apply to {short_name}: it ships inside the firmware, so its \
         version axis is the firmware's -- select it with --fw"
    )]
    FirmwareExec {
        /// The title the flag was passed for.
        short_name: String,
    },
}

/// Render a version list for a refusal, or `(none)` when empty.
fn render_list(versions: &[String]) -> String {
    if versions.is_empty() {
        "(none)".to_string()
    } else {
        versions.join(", ")
    }
}

/// Resolve `--fw` against the store.
///
/// # Errors
///
/// Every [`FirmwareSelectError`]:
///
/// - a named version that is not installed;
/// - zero or several candidates with no flag;
/// - a record whose tree is gone or cannot be probed.
pub(crate) fn select_firmware(
    inventory: &StoreInventory,
    asked: Option<&str>,
    disable_env: &'static str,
) -> Result<FirmwareEntry, FirmwareSelectError> {
    let root = inventory.root().display().to_string();
    let entry = match asked {
        Some(version) => {
            inventory
                .firmware(version)
                .ok_or_else(|| FirmwareSelectError::NotInstalled {
                    asked: version.to_string(),
                    root: root.clone(),
                    installed: inventory.firmware_versions(),
                })?
        }
        None => inventory.sole_firmware().ok_or_else(|| {
            let installed = inventory.firmware_versions();
            if installed.is_empty() {
                FirmwareSelectError::NoneInstalled {
                    root: root.clone(),
                    disable_env,
                }
            } else {
                FirmwareSelectError::Ambiguous {
                    root: root.clone(),
                    installed,
                }
            }
        })?,
    };
    let dev_flash = entry.dev_flash_dir();
    match dir_exists(&dev_flash) {
        Ok(true) => Ok(entry.clone()),
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

/// Resolve `--game-ver` against one title's store entries.
///
/// # Errors
///
/// Every [`GameVersionSelectError`] except
/// [`GameVersionSelectError::FirmwareExec`], which the caller raises
/// before a store lookup happens.
pub(crate) fn select_game_version(
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
        Some(BASE_VERSION) => Ok(GameVersion::Base),
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
