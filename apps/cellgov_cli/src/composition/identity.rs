//! The identity triple a composed boot writes into every machine
//! artifact it produces.
//!
//! Each half comes from the store, held against the tree it names. A
//! firmware entry names the version and the PUP it was installed from.
//! The `firmware.toml` inside its tree names the PUP-header
//! `image_version`. A title entry names the version its record holds.
//! The PARAM.SFO inside its tree says which key named it. A half the
//! store did not compose -- an unmanaged `--firmware-dir` tree, a title
//! with no store entry -- is `None`, because no version key names it.

use std::path::{Path, PathBuf};

use cellgov_compare::{AppVersion, BootOverrides, FirmwareIdentity, GameIdentity, RunIdentity};
use cellgov_install::manifest::MANIFEST_FILE;
use cellgov_install::param_sfo::{self, SfoVersionKey};

use super::compose::{GameChoice, StoredGame};
use super::inventory::FirmwareEntry;
use super::select::{FirmwareChoice, GameVersion};
use cellgov_boot::manifest::BASE_GAME_VER;

/// Why a composed boot could not name the identity triple it runs.
#[derive(Debug, thiserror::Error)]
pub(crate) enum IdentityError {
    /// The selected firmware entry's identity could not be read, so
    /// the run cannot name the PUP it tests against. Boxed: its
    /// mismatch variant carries two version/digest pairs.
    #[error("reading the selected firmware's identity: {0}")]
    Firmware(#[from] Box<FirmwareIdentityError>),
    /// The selected title tree's version could not be read, so the run
    /// cannot name the content it tests.
    #[error("reading the selected title's identity: {0}")]
    Game(#[from] GameIdentityError),
}

/// Why the selected firmware's identity could not be read.
#[derive(Debug, thiserror::Error)]
pub(crate) enum FirmwareIdentityError {
    /// The manifest inside the entry's tree could not be read.
    #[error("read {}: {source}", path.display())]
    Read {
        /// The manifest file.
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    /// The manifest does not parse under this build's schema.
    #[error("{}: {source}", path.display())]
    Parse {
        /// The manifest file.
        path: PathBuf,
        /// Boxed: the TOML parse error it wraps is far larger than
        /// every other variant.
        #[source]
        source: Box<cellgov_install::manifest::ManifestError>,
    },
    /// The manifest names a different install than the store's record
    /// for the entry. Boxed: two version/digest pairs are far larger
    /// than every other variant.
    #[error(
        "{} names firmware {} from PUP {}, but the store's record files this entry as {} from \
         PUP {}; one of the two is stale, so the run cannot name the PUP it is testing \
         against. Reinstall the firmware",
        .0.path.display(), .0.found_version, .0.found_sha256, .0.entry_version, .0.entry_sha256
    )]
    Mismatch(Box<FirmwareClaims>),
}

/// What the record and the manifest each say the entry was installed
/// from.
#[derive(Debug)]
pub(crate) struct FirmwareClaims {
    /// The manifest file.
    pub path: PathBuf,
    /// Version the record declares.
    pub entry_version: String,
    /// Version the manifest declares.
    pub found_version: String,
    /// Source digest the record declares.
    pub entry_sha256: String,
    /// Source digest the manifest declares.
    pub found_sha256: String,
}

/// Why the selected title tree's version could not be read.
#[derive(Debug, thiserror::Error)]
pub(crate) enum GameIdentityError {
    /// The PARAM.SFO inside the tree could not be read.
    #[error("read {}: {source}", path.display())]
    Read {
        /// The PARAM.SFO file.
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    /// The PARAM.SFO does not parse.
    #[error("{}: {source}", path.display())]
    Parse {
        /// The PARAM.SFO file.
        path: PathBuf,
        #[source]
        source: param_sfo::SfoError,
    },
    /// The tree's PARAM.SFO names a different version than the store's
    /// record for the entry.
    #[error(
        "{} names {}, but the store's record files this tree as version {recorded:?}; one of \
         the two is stale, so the run cannot name the content it is testing. Reinstall the \
         title",
        path.display(), render_named_version(found)
    )]
    Mismatch {
        /// The PARAM.SFO file.
        path: PathBuf,
        /// Version the record declares.
        recorded: String,
        /// What the tree's PARAM.SFO names, under its key.
        found: Option<AppVersion>,
    },
}

fn render_named_version(found: &Option<AppVersion>) -> String {
    found
        .as_ref()
        .map_or_else(|| "no version key".to_string(), ToString::to_string)
}

/// Build the identity triple for a composed boot.
///
/// # Errors
///
/// [`IdentityError::Firmware`] when a managed firmware entry's
/// manifest:
///
/// - cannot be read;
/// - does not parse;
/// - names another install than the store's record does.
///
/// [`IdentityError::Game`] on the same three failures of the selected
/// title tree's PARAM.SFO.
pub(crate) fn run_identity(
    firmware: &FirmwareChoice,
    game: &GameChoice,
) -> Result<RunIdentity, IdentityError> {
    Ok(RunIdentity {
        firmware: match firmware {
            FirmwareChoice::Managed(managed) => {
                Some(firmware_identity(&managed.entry).map_err(Box::new)?)
            }
            FirmwareChoice::Unmanaged { .. } | FirmwareChoice::None => None,
        },
        game: match game {
            GameChoice::Stored(stored) => Some(game_identity(stored)?),
            GameChoice::Firmware { .. } | GameChoice::Unstored => None,
        },
        // The store composes no override. `try_resolve_composition` in
        // `boot_cmd` replaces this empty set with the boot's own flags.
        overrides: BootOverrides::default(),
    })
}

fn firmware_identity(entry: &FirmwareEntry) -> Result<FirmwareIdentity, FirmwareIdentityError> {
    let path = entry.dev_flash_dir().join(MANIFEST_FILE);
    let manifest = parse_manifest(&path)?;
    // One install writes both the record and the manifest from one PUP,
    // so a correct store keeps the two in agreement.
    let found_sha256 = manifest.firmware.pup_sha256.to_hex();
    if manifest.firmware.version != entry.version || found_sha256 != entry.pup_sha256 {
        return Err(FirmwareIdentityError::Mismatch(Box::new(FirmwareClaims {
            path,
            entry_version: entry.version.clone(),
            found_version: manifest.firmware.version,
            entry_sha256: entry.pup_sha256.clone(),
            found_sha256,
        })));
    }
    Ok(FirmwareIdentity {
        version: entry.version.clone(),
        image_version: manifest.firmware.image_version,
        pup_sha256: entry.pup_sha256.clone(),
    })
}

fn parse_manifest(
    path: &Path,
) -> Result<cellgov_install::manifest::FirmwareManifest, FirmwareIdentityError> {
    let text = std::fs::read_to_string(path).map_err(|source| FirmwareIdentityError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    cellgov_install::manifest::parse_manifest(&text).map_err(|source| {
        FirmwareIdentityError::Parse {
            path: path.to_path_buf(),
            source: Box::new(source),
        }
    })
}

/// The game half: the selected version, and the PARAM.SFO version of
/// the tree that leads the executable probe.
///
/// That tree is the update's when one is selected, and the base's
/// otherwise. This names the composed version. The executable that
/// loads can still come from another tree: the probe falls back to the
/// base when the update holds no candidate, and an explicit path
/// bypasses the probe.
fn game_identity(stored: &StoredGame) -> Result<GameIdentity, GameIdentityError> {
    let (version, recorded, path) = match &stored.version {
        GameVersion::Base => (
            BASE_GAME_VER.to_string(),
            &stored.base.version,
            stored.base.param_sfo_path(),
        ),
        GameVersion::Update(v) => {
            let update = stored
                .update
                .as_ref()
                .expect("invariant: resolve_game carries the entry of the selected update");
            (
                format!("update:{v}"),
                &update.version,
                update.param_sfo_path(),
            )
        }
    };
    let app_version = tree_app_version(path, recorded)?;
    Ok(GameIdentity {
        title_id: stored.title_id.clone(),
        version,
        app_version,
    })
}

/// The version a tree's PARAM.SFO names, under its key, held against
/// the version the tree's record holds.
///
/// `Ok(None)` is a table that names no version over a record that
/// holds the empty string, the pair a base install writes for such a
/// table.
///
/// # Errors
///
/// - [`GameIdentityError::Read`] when the table cannot be read.
/// - [`GameIdentityError::Parse`] when the table does not parse.
/// - [`GameIdentityError::Mismatch`] when the version the table names
///   is not `recorded`.
pub(crate) fn tree_app_version(
    path: PathBuf,
    recorded: &str,
) -> Result<Option<AppVersion>, GameIdentityError> {
    let app_version = read_app_version(&path)?;
    // The install wrote the record's version from this same table, so a
    // correct store keeps the two in agreement.
    if app_version.as_ref().map_or("", AppVersion::value) != recorded {
        return Err(GameIdentityError::Mismatch {
            path,
            recorded: recorded.to_string(),
            found: app_version,
        });
    }
    Ok(app_version)
}

fn read_app_version(path: &Path) -> Result<Option<AppVersion>, GameIdentityError> {
    let bytes = std::fs::read(path).map_err(|source| GameIdentityError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    let sfo = param_sfo::parse(&bytes).map_err(|source| GameIdentityError::Parse {
        path: path.to_path_buf(),
        source,
    })?;
    Ok(sfo.named_version().map(|(key, value)| match key {
        SfoVersionKey::AppVer => AppVersion::AppVer(value.to_string()),
        SfoVersionKey::Version => AppVersion::SfoVersion(value.to_string()),
    }))
}

#[cfg(test)]
#[path = "tests/identity_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/game_identity_tests.rs"]
mod game_identity_tests;
