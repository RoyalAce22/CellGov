//! The store root the read commands resolve, and the view they read
//! from it.

use std::path::{Path, PathBuf};

use crate::cli::exit::CommandError;
use cellgov_boot::manifest::TitleRegistry;
use cellgov_install::store::inventory::StoreInventory;

use super::collect::StoreView;

/// Reads the store and title registry under `root`.
///
/// # Errors
///
/// Returns an error when the inventory or title registry cannot be read.
pub(super) fn view(root: &Path) -> Result<StoreView, CommandError> {
    let inventory =
        StoreInventory::read(root).map_err(|error| CommandError::failed(error.to_string()))?;
    let registry_dir = Path::new(crate::cli::title::DEFAULT_TITLE_REGISTRY_DIR);
    let registry = TitleRegistry::scan_dir(registry_dir)
        .map_err(|error| CommandError::failed(format!("title registry: {error}")))?;
    // The registry directory resolves against the working directory, and
    // an absent one reads as a registry that declares nothing.
    if registry.is_empty() && !registry_dir.is_dir() {
        eprintln!(
            "warning: no title registry directory {} under the working directory; nothing \
             declares a title, so every installed title reads as an orphan and no cell is named",
            registry_dir.display()
        );
    }
    Ok(StoreView {
        root: root.to_path_buf(),
        inventory,
        registry,
        fixtures: crate::paths::fixtures_dir(&crate::paths::workspace_root()),
    })
}

/// Resolves the store root for read commands.
///
/// # Errors
///
/// Returns an error when the VFS root override is empty.
pub(crate) fn store_root(vfs_flag: Option<&Path>) -> Result<PathBuf, CommandError> {
    Ok(crate::cli::keys::install_root_of(
        &crate::cli::title::resolve_ps3_vfs_root(vfs_flag)?,
    ))
}
