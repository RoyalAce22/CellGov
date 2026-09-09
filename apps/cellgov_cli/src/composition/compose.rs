//! The guest-visible tree an identity triple (firmware, title, game
//! version) produces.
//!
//! Composition is path arithmetic over the store's install records,
//! plus one existence probe per root. It copies and merges nothing: an
//! update that patches a base becomes a second mount root ahead of the
//! base.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use cellgov_compare::RunIdentity;
use cellgov_install::store::TitleTree;
use cellgov_ps3_abi::dev_flash::GUEST_FLASH_MOUNT;
use cellgov_ps3_abi::title_tree::DISC_GAME_DIR;

use super::identity::{run_identity, IdentityError};
use super::inventory::{dir_exists, BaseEntry, InventoryError, StoreInventory, UpdateEntry};
use super::select::{
    select_firmware, select_game_version, FirmwareChoice, FirmwareSelectError, GameVersion,
    GameVersionSelectError,
};
use crate::game::manifest::{GameSource, ResolveEbootError, TitleManifest};

/// Guest prefix a title's disc tree mounts under, joined with the
/// title id.
const GUEST_BDVD: &str = "/dev_bdvd";

/// Guest prefix a title's HDD game tree mounts under, joined with the
/// title id.
const GUEST_GAME: &str = "/dev_hdd0/game";

/// Guest path of the one modeled user profile's license directory.
/// Names the same directory as `StoreLayout::live_exdata_dir`.
const GUEST_EXDATA: &str = "/dev_hdd0/home/00000001/exdata";

/// Where a disc tree holds its executable, under the entry directory.
const DISC_USRDIR: [&str; 2] = [DISC_GAME_DIR, "USRDIR"];

/// Where an HDD game tree holds its executable.
const GAME_USRDIR: &str = "USRDIR";

/// Why a boot could not be composed.
#[derive(Debug, thiserror::Error)]
pub(crate) enum ComposeError {
    /// The store's records could not be read.
    #[error("{0}")]
    Inventory(#[from] InventoryError),
    /// `--fw` did not resolve to one installed firmware.
    #[error("{0}")]
    Firmware(#[from] FirmwareSelectError),
    /// `--game-ver` did not resolve to one installed version.
    #[error("{0}")]
    GameVersion(#[from] GameVersionSelectError),
    /// A composed half's tree disagrees with the record that names it,
    /// or cannot be read, so the run cannot name what it tests.
    #[error("{0}")]
    Identity(#[from] IdentityError),
    /// A title with no store entry could not derive its executable
    /// directory from the VFS root. Boxed: its not-found variant
    /// carries four probe lists.
    #[error("{0}")]
    ResolveEboot(#[from] Box<ResolveEbootError>),
    /// `--game-ver` was passed for a title the store does not hold.
    #[error(
        "--game-ver names an installed version, and {title_id} has no store entry under \
         {root}; install it first, or drop the flag"
    )]
    TitleNotInStore {
        /// The title the store was searched for.
        title_id: String,
        /// The VFS root the store was read under.
        root: String,
    },
    /// A firmware-relative executable path with no firmware entry to
    /// resolve it against.
    #[error(
        "{short_name} names its executable at {dir}, relative to a firmware entry, and this \
         run selected no managed firmware. Pick one with --fw; --firmware-dir names a module \
         directory, which is not the entry root this path is relative to"
    )]
    FirmwareRelativeWithoutEntry {
        /// The title being booted.
        short_name: String,
        /// The path the manifest named.
        dir: String,
    },
    /// A record names a tree that is gone.
    #[error("{title_id} {version} is recorded but its tree at {dir} is missing; reinstall it")]
    TreeMissing {
        /// The title whose record was read.
        title_id: String,
        /// The version whose record was read.
        version: String,
        /// The directory the record named.
        dir: String,
    },
    /// A record names a tree that could be neither read nor shown
    /// absent.
    #[error(
        "{title_id} {version} is recorded but its tree at {dir} could not be probed: {source}"
    )]
    TreeUnreadable {
        /// The title whose record was read.
        title_id: String,
        /// The version whose record was read.
        version: String,
        /// The directory the record named.
        dir: String,
        /// The underlying failure.
        #[source]
        source: std::io::Error,
    },
    /// An exdata directory could not be enumerated or read.
    #[error("reading the license directory {dir}: {source}")]
    ReadExdata {
        /// The directory that could not be read.
        dir: String,
        /// The underlying failure.
        #[source]
        source: std::io::Error,
    },
    /// Two titles hold a same-named license file with different bytes,
    /// so the union has no one answer for that name.
    #[error(
        "license file {filename} differs between {first} and {second}; the composed license \
         directory is a content union, so two files of one name must be the same bytes. \
         Reinstall whichever title holds the stale copy"
    )]
    ExdataConflict {
        /// The colliding filename.
        filename: String,
        /// The root that supplied the name first.
        first: String,
        /// The root that disagreed.
        second: String,
    },
}

/// One composed mount: a guest prefix and the host roots that answer
/// it, in probe order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ComposedMount {
    /// Guest path prefix.
    pub prefix: String,
    /// Host roots, first hit wins.
    pub roots: Vec<PathBuf>,
}

/// The store entries one composition rests on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StoredGame {
    /// The store key.
    pub title_id: String,
    /// The selected version.
    pub version: GameVersion,
    /// The base entry every composition rests on.
    pub base: BaseEntry,
    /// The selected update, when one was selected.
    pub update: Option<UpdateEntry>,
}

/// Which content the title's executable and data come from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum GameChoice {
    /// Composed from the store.
    Stored(Box<StoredGame>),
    /// The executable ships inside the firmware, so the title has no
    /// version axis of its own.
    Firmware {
        /// Where the executable sits, resolved against the selected
        /// firmware entry when the manifest named a relative path.
        dir: PathBuf,
        /// True when the manifest's path was kept as written because
        /// no managed firmware was selected to resolve it against.
        unmanaged_path: bool,
    },
    /// The title has no store entry: nothing is composed for it, and
    /// the executable is resolved from the VFS root the caller named.
    Unstored,
}

/// The firmware, the content, and the guest tree they produce.
#[derive(Debug, Clone)]
pub(crate) struct BootComposition {
    /// Which firmware answers `/dev_flash`.
    pub firmware: FirmwareChoice,
    /// Which content answers the title's mounts.
    pub game: GameChoice,
    /// Mounts to register before the title's own, so a composed mount
    /// is never shadowed by a broader manifest prefix.
    pub mounts: Vec<ComposedMount>,
    /// Directories the EBOOT is probed in, first hit wins.
    pub eboot_dirs: Vec<PathBuf>,
    /// Updates whose declared minimum firmware the selection does not
    /// meet. The boot reports these and continues.
    pub understated_firmware: Vec<UnderstatedFirmware>,
    /// The identity triple every machine artifact this boot writes
    /// embeds.
    pub identity: RunIdentity,
}

/// An update whose declared minimum firmware the selection does not
/// meet, or whose declared minimum could not be ordered against it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct UnderstatedFirmware {
    /// The update version that declared the minimum.
    pub update: String,
    /// The version the update's metadata declared.
    pub declared: String,
    /// The firmware the boot selected.
    pub selected: String,
    /// True when a string did not parse as a version, so the two were
    /// never ordered.
    pub incomparable: bool,
}

/// What a caller passes to [`compose_boot`].
pub(crate) struct ComposeInputs<'a> {
    /// The title being booted.
    pub title: &'a TitleManifest,
    /// The `dev_hdd0` mount the caller named.
    pub vfs_root: &'a Path,
    /// The directory holding the store and its records, one level
    /// above [`Self::vfs_root`].
    pub install_root: &'a Path,
    /// `--fw`.
    pub fw: Option<&'a str>,
    /// `--game-ver`.
    pub game_ver: Option<&'a str>,
    /// `--firmware-dir`, already validated as an existing directory.
    pub firmware_dir: Option<&'a Path>,
    /// A boot asked to run with no firmware at all.
    pub no_firmware: bool,
    /// Variable that asks for a firmware-free boot, named in the
    /// nothing-installed refusal.
    pub disable_env: &'static str,
}

/// Resolve the selections and build the guest tree they produce.
///
/// # Errors
///
/// Every [`ComposeError`]:
///
/// - the record walk;
/// - both selection contracts;
/// - a record whose tree is gone or cannot be probed;
/// - a composed half whose tree cannot be read against its record, or
///   disagrees with it;
/// - a license-directory union that holds two different files of one
///   name.
pub(crate) fn compose_boot(inputs: &ComposeInputs<'_>) -> Result<BootComposition, ComposeError> {
    let inventory = StoreInventory::read(inputs.install_root)?;
    let firmware = match (inputs.firmware_dir, inputs.no_firmware) {
        (Some(dir), _) => FirmwareChoice::Unmanaged {
            dir: dir.to_path_buf(),
        },
        (None, true) => FirmwareChoice::None,
        (None, false) => {
            FirmwareChoice::Managed(select_firmware(&inventory, inputs.fw, inputs.disable_env)?)
        }
    };

    let mut mounts = Vec::new();
    if let FirmwareChoice::Managed(entry) = &firmware {
        mounts.push(ComposedMount {
            prefix: GUEST_FLASH_MOUNT.to_string(),
            roots: vec![entry.dev_flash_dir()],
        });
    }

    let game = resolve_game(inputs, &inventory, &firmware)?;
    let mut understated_firmware = Vec::new();
    let eboot_dirs = match &game {
        GameChoice::Firmware { dir, .. } => vec![dir.clone()],
        GameChoice::Unstored => inputs
            .title
            .eboot_dirs(inputs.vfs_root)
            .map_err(|e| ComposeError::ResolveEboot(Box::new(e)))?,
        GameChoice::Stored(stored) => {
            let StoredGame {
                title_id,
                base,
                update,
                ..
            } = stored.as_ref();
            check_tree(title_id, &base.version, &base.dir)?;
            if let Some(u) = update {
                check_tree(title_id, &u.version, &u.dir)?;
                if let Some(note) = firmware_shortfall(u, &firmware) {
                    understated_firmware.push(note);
                }
            }
            mounts.extend(title_mounts(title_id, base, update.as_ref()));
            title_eboot_dirs(base, update.as_ref())
        }
    };

    let exdata_roots = inventory.exdata_roots()?;
    check_exdata_union(&exdata_roots)?;
    if !exdata_roots.is_empty() {
        mounts.push(ComposedMount {
            prefix: GUEST_EXDATA.to_string(),
            roots: exdata_roots,
        });
    }

    let identity = run_identity(&firmware, &game)?;
    Ok(BootComposition {
        firmware,
        game,
        mounts,
        eboot_dirs,
        understated_firmware,
        identity,
    })
}

/// Apply the selection contract to the title's own version axis.
fn resolve_game(
    inputs: &ComposeInputs<'_>,
    inventory: &StoreInventory,
    firmware: &FirmwareChoice,
) -> Result<GameChoice, ComposeError> {
    if let GameSource::FirmwareExec { dir } = &inputs.title.source {
        if inputs.game_ver.is_some() {
            return Err(GameVersionSelectError::FirmwareExec {
                short_name: inputs.title.name().to_string(),
            }
            .into());
        }
        // A relative path resolves against the selected firmware
        // entry, so one manifest boots against every installed
        // firmware. Anything else keeps the path the manifest named.
        return match firmware {
            FirmwareChoice::Managed(entry) if dir.is_relative() => Ok(GameChoice::Firmware {
                dir: entry.entry_dir.join(dir),
                unmanaged_path: false,
            }),
            // An absolute path names its own tree, so it needs no
            // entry to resolve against.
            _ if !dir.is_relative() => Ok(GameChoice::Firmware {
                dir: dir.clone(),
                unmanaged_path: true,
            }),
            _ => Err(ComposeError::FirmwareRelativeWithoutEntry {
                short_name: inputs.title.name().to_string(),
                dir: dir.display().to_string(),
            }),
        };
    }
    let title_id = &inputs.title.content_id;
    let Some(entry) = inventory.title(title_id) else {
        if inputs.game_ver.is_some() {
            return Err(ComposeError::TitleNotInStore {
                title_id: title_id.clone(),
                root: inventory.root().display().to_string(),
            });
        }
        return Ok(GameChoice::Unstored);
    };
    let version = select_game_version(entry, inputs.game_ver)?;
    let update = match &version {
        GameVersion::Base => None,
        GameVersion::Update(v) => entry.updates.get(v).cloned(),
    };
    Ok(GameChoice::Stored(Box::new(StoredGame {
        title_id: title_id.clone(),
        version,
        base: entry
            .base
            .clone()
            .expect("invariant: select_game_version refuses an entry with no base"),
        update,
    })))
}

/// The composition table's title rows.
///
/// A disc base keeps its `/dev_bdvd` mount whatever is selected: the
/// disc is still in the drive. For a disc title, a selected update
/// answers `/dev_hdd0/game` alone. For an HDD title, it answers ahead
/// of the base, which is how a patch PKG lands on hardware.
fn title_mounts(
    title_id: &str,
    base: &BaseEntry,
    update: Option<&UpdateEntry>,
) -> Vec<ComposedMount> {
    let mut mounts = Vec::new();
    let game_prefix = format!("{GUEST_GAME}/{title_id}");
    match base.tree {
        TitleTree::Disc => {
            mounts.push(ComposedMount {
                prefix: format!("{GUEST_BDVD}/{title_id}"),
                roots: vec![base.dir.clone()],
            });
            if let Some(u) = update {
                mounts.push(ComposedMount {
                    prefix: game_prefix,
                    roots: vec![u.dir.clone()],
                });
            }
        }
        TitleTree::Game => {
            let mut roots = Vec::new();
            if let Some(u) = update {
                roots.push(u.dir.clone());
            }
            roots.push(base.dir.clone());
            mounts.push(ComposedMount {
                prefix: game_prefix,
                roots,
            });
        }
    }
    mounts
}

/// Directories the EBOOT is probed in, update first.
///
/// A patch PKG carries a complete game directory with its own
/// executable, and the console runs that one rather than the base's.
fn title_eboot_dirs(base: &BaseEntry, update: Option<&UpdateEntry>) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(u) = update {
        dirs.push(u.dir.join(GAME_USRDIR));
    }
    dirs.push(match base.tree {
        TitleTree::Disc => DISC_USRDIR.iter().fold(base.dir.clone(), |d, p| d.join(p)),
        TitleTree::Game => base.dir.join(GAME_USRDIR),
    });
    dirs
}

/// Refuse a union in which one filename has two different contents.
///
/// Two titles that share a license each hold their own copy, so one
/// name in two roots is normal. Two different byte strings under one
/// name mean one copy is stale, and the union would serve whichever
/// root comes first.
fn check_exdata_union(roots: &[PathBuf]) -> Result<(), ComposeError> {
    let mut seen: BTreeMap<String, (PathBuf, Vec<u8>)> = BTreeMap::new();
    for root in roots {
        let entries = std::fs::read_dir(root).map_err(|source| ComposeError::ReadExdata {
            dir: root.display().to_string(),
            source,
        })?;
        for entry in entries {
            let entry = entry.map_err(|source| ComposeError::ReadExdata {
                dir: root.display().to_string(),
                source,
            })?;
            let path = entry.path();
            // Skipping an entry that cannot be probed would leave a
            // name the union serves without a byte comparison.
            let meta = std::fs::metadata(&path).map_err(|source| ComposeError::ReadExdata {
                dir: path.display().to_string(),
                source,
            })?;
            if !meta.is_file() {
                continue;
            }
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            let bytes = std::fs::read(&path).map_err(|source| ComposeError::ReadExdata {
                dir: path.display().to_string(),
                source,
            })?;
            match seen.get(&name) {
                Some((first, first_bytes)) if *first_bytes != bytes => {
                    return Err(ComposeError::ExdataConflict {
                        filename: name,
                        first: first.display().to_string(),
                        second: root.display().to_string(),
                    });
                }
                Some(_) => {}
                None => {
                    seen.insert(name, (root.clone(), bytes));
                }
            }
        }
    }
    Ok(())
}

fn check_tree(title_id: &str, version: &str, dir: &Path) -> Result<(), ComposeError> {
    match dir_exists(dir) {
        Ok(true) => Ok(()),
        Ok(false) => Err(ComposeError::TreeMissing {
            title_id: title_id.to_string(),
            version: version.to_string(),
            dir: dir.display().to_string(),
        }),
        Err(source) => Err(ComposeError::TreeUnreadable {
            title_id: title_id.to_string(),
            version: version.to_string(),
            dir: dir.display().to_string(),
            source,
        }),
    }
}

/// Whether the selected firmware is older than the update declared it
/// needs.
fn firmware_shortfall(
    update: &UpdateEntry,
    firmware: &FirmwareChoice,
) -> Option<UnderstatedFirmware> {
    let declared = update.min_system_ver.as_deref()?;
    let selected = firmware.version()?;
    let note = |incomparable| UnderstatedFirmware {
        update: update.version.clone(),
        declared: declared.to_string(),
        selected: selected.to_string(),
        incomparable,
    };
    match (version_key(declared), version_key(selected)) {
        (Some(want), Some(have)) if have < want => Some(note(false)),
        (Some(_), Some(_)) => None,
        _ => Some(note(true)),
    }
}

/// A Sony version string as a comparable `(major, minor)` pair.
///
/// `4.93` and `04.9300` are one version written two ways: the
/// console's `version.txt` form and the update metadata's. This
/// right-pads the fraction to the metadata form's four digits, so both
/// normalize to `(4, 9300)`.
fn version_key(s: &str) -> Option<(u32, u32)> {
    let (major, minor) = s.split_once('.')?;
    if minor.is_empty() || minor.len() > 4 || !minor.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let mut padded = minor.to_string();
    while padded.len() < 4 {
        padded.push('0');
    }
    Some((major.parse().ok()?, padded.parse().ok()?))
}

#[cfg(test)]
#[path = "tests/compose_tests.rs"]
mod tests;
