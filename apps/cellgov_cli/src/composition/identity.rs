//! The identity triple a composed boot writes into every machine
//! artifact it produces.
//!
//! Each half comes from the store. A firmware entry names the version
//! and the PUP it was installed from. The `firmware.toml` inside its
//! tree names the PUP-header `image_version`. A half the store did not
//! compose -- an unmanaged `--firmware-dir` tree, a title with no store
//! entry -- is `None`, because no version key names it.

use std::path::{Path, PathBuf};

use cellgov_compare::{FirmwareIdentity, GameIdentity, RunIdentity};
use cellgov_install::manifest::MANIFEST_FILE;

use super::compose::{GameChoice, StoredGame};
use super::inventory::FirmwareEntry;
use super::select::{FirmwareChoice, GameVersion};
use crate::game::manifest::BASE_GAME_VER;

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

/// Build the triple for a composed boot.
///
/// # Errors
///
/// [`FirmwareIdentityError`] when a managed firmware entry's manifest:
///
/// - cannot be read;
/// - does not parse;
/// - names another install than the store's record does.
pub(crate) fn run_identity(
    firmware: &FirmwareChoice,
    game: &GameChoice,
) -> Result<RunIdentity, FirmwareIdentityError> {
    Ok(RunIdentity {
        firmware: match firmware {
            FirmwareChoice::Managed(entry) => Some(firmware_identity(entry)?),
            FirmwareChoice::Unmanaged { .. } | FirmwareChoice::None => None,
        },
        game: match game {
            GameChoice::Stored(stored) => Some(game_identity(stored)),
            GameChoice::Firmware { .. } | GameChoice::Unstored => None,
        },
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

/// The game half: the selected version, and the `APP_VER` of the tree
/// that leads the executable probe.
///
/// That tree is the update's when one is selected, and the base's
/// otherwise. This names the composed version. The executable that
/// loads can still come from another tree: the probe falls back to the
/// base when the update holds no candidate, and an explicit path
/// bypasses the probe.
fn game_identity(stored: &StoredGame) -> GameIdentity {
    let (version, app_ver) = match &stored.version {
        GameVersion::Base => (BASE_GAME_VER.to_string(), stored.base.app_ver.clone()),
        GameVersion::Update(v) => (
            format!("update:{v}"),
            stored
                .update
                .as_ref()
                .expect("invariant: resolve_game carries the entry of the selected update")
                .version
                .clone(),
        ),
    };
    GameIdentity {
        title_id: stored.title_id.clone(),
        version,
        app_ver,
    }
}

#[cfg(test)]
#[path = "tests/identity_tests.rs"]
mod tests;
