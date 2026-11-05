//! The installed titles a census reads, and the plaintext ELF images
//! their install trees hold.

use std::io::Read;
use std::path::{Path, PathBuf};

use cellgov_boot::manifest::{TitleManifest, TitleRegistry};
use cellgov_install::self_image::is_sce_wrapped;
use cellgov_install::store::StoreInventory;
use cellgov_ps3_abi::format::elf::ELF_MAGIC;

use crate::cli::exit::CommandError;
use crate::cli::parse::TitleCensusScope;
use crate::cli::self_load::decrypt_ppu_self;

/// What a census reads: the titles in scope and the store that holds them.
pub(crate) struct CensusInputs {
    pub vfs_root: PathBuf,
    /// The titles in scope, in registry order.
    pub titles: Vec<TitleManifest>,
    pub inventory: StoreInventory,
}

/// Resolve the titles that `scope` names and the store that holds them.
///
/// `command` prefixes each error message.
///
/// # Errors
///
/// - The title registry or the store is unreadable.
/// - `scope` names a title that the registry does not hold.
pub(crate) fn census_inputs(
    scope: &TitleCensusScope,
    vfs_flag: Option<&Path>,
    command: &str,
) -> Result<CensusInputs, CommandError> {
    let vfs_root = crate::cli::title::resolve_ps3_vfs_root(vfs_flag)?;
    let registry = TitleRegistry::scan_dir(&crate::cli::store::registry_dir())
        .map_err(|error| CommandError::failed(format!("{command}: title registry: {error}")))?;
    let titles: Vec<TitleManifest> = match &scope.title {
        Some(name) => vec![registry
            .by_short_name(name)
            .ok_or_else(|| {
                CommandError::failed(format!(
                    "{command}: unknown title '{name}'. Known titles: {}",
                    registry.known_names_csv()
                ))
            })?
            .clone()],
        None => registry.iter().cloned().collect(),
    };
    let inventory = StoreInventory::read(&crate::cli::keys::install_root_of(&vfs_root))
        .map_err(|error| CommandError::failed(format!("{command}: store: {error}")))?;
    Ok(CensusInputs {
        vfs_root,
        titles,
        inventory,
    })
}

/// The files of one title's install trees, in path order.
pub(crate) struct TitleFiles {
    /// Every regular file under the trees.
    pub files: Vec<PathBuf>,
    /// Each directory that the walk could not list, and why.
    pub unreadable: Vec<(PathBuf, String)>,
}

/// Why a census reads nothing of a title.
#[derive(Debug, thiserror::Error)]
pub(crate) enum TitleSkip {
    #[error("ships inside the firmware")]
    ShipsInFirmware,
    /// No install tree of the title exists.
    #[error("not installed")]
    NotInstalled,
}

/// Why [`read_image`] refuses one file.
#[derive(Debug, thiserror::Error)]
pub(crate) enum ImageReadError {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    /// `decrypt_ppu_self` refused the SCE container.
    #[error(transparent)]
    Decrypt(CommandError),
}

/// The files of every installed version of `title`, in path order.
///
/// The installed versions are the base tree and each update tree.
///
/// # Errors
///
/// The [`TitleSkip`] reason when the census can read nothing of the title.
pub(crate) fn title_files(
    title: &TitleManifest,
    inventory: &StoreInventory,
    vfs_root: &Path,
) -> Result<TitleFiles, TitleSkip> {
    if title.ships_in_firmware() {
        return Err(TitleSkip::ShipsInFirmware);
    }
    let dirs: Vec<PathBuf> = match inventory.title(&title.content_id) {
        Some(entry) => entry
            .base
            .iter()
            .map(|base| base.dir.clone())
            .chain(entry.updates.values().map(|update| update.dir.clone()))
            .collect(),
        None => title.eboot_dirs(vfs_root).unwrap_or_default(),
    };
    let dirs: Vec<PathBuf> = dirs.into_iter().filter(|d| d.is_dir()).collect();
    if dirs.is_empty() {
        return Err(TitleSkip::NotInstalled);
    }
    let mut found = TitleFiles {
        files: Vec::new(),
        unreadable: Vec::new(),
    };
    for dir in &dirs {
        walk(dir, &mut found.files, &mut found.unreadable);
    }
    found.files.sort();
    Ok(found)
}

/// Every regular file under `dir`, recursively.
fn walk(dir: &Path, files: &mut Vec<PathBuf>, unreadable: &mut Vec<(PathBuf, String)>) {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) => {
            unreadable.push((dir.to_path_buf(), error.to_string()));
            return;
        }
    };
    for entry in entries.flatten() {
        let path = entry.path();
        match entry.file_type() {
            Ok(kind) if kind.is_dir() => walk(&path, files, unreadable),
            Ok(kind) if kind.is_file() => files.push(path),
            _ => {}
        }
    }
}

/// The plaintext ELF bytes of `path`, or `None` for a file that is
/// neither an ELF nor an SCE container.
///
/// # Errors
///
/// - [`ImageReadError::Io`] when the file system refuses the open or the read.
/// - [`ImageReadError::Decrypt`] when the SCE container does not decrypt.
pub(crate) fn read_image(path: &Path, vfs_root: &Path) -> Result<Option<Vec<u8>>, ImageReadError> {
    let mut head = [0u8; 4];
    let read = std::fs::File::open(path).and_then(|mut file| file.read(&mut head))?;
    let head = &head[..read];
    if head != ELF_MAGIC && !is_sce_wrapped(head) {
        return Ok(None);
    }
    let bytes = std::fs::read(path)?;
    if head == ELF_MAGIC {
        return Ok(Some(bytes));
    }
    decrypt_ppu_self(&bytes, &path.display().to_string(), vfs_root)
        .map(Some)
        .map_err(ImageReadError::Decrypt)
}
