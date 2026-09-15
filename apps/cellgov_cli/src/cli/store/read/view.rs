//! The store root the read commands resolve, and the view they read
//! from it.

use std::path::{Path, PathBuf};

use crate::cli::exit::die;
use crate::composition::inventory::StoreInventory;
use cellgov_boot::manifest::TitleRegistry;

use super::collect::StoreView;

/// Read the store under `root`, or die naming what refused.
pub(super) fn view(root: &Path) -> StoreView {
    let inventory = StoreInventory::read(root).unwrap_or_else(|e| die(&e.to_string()));
    let registry_dir = Path::new(crate::cli::title::DEFAULT_TITLE_REGISTRY_DIR);
    let registry = TitleRegistry::scan_dir(registry_dir)
        .unwrap_or_else(|e| die(&format!("title registry: {e}")));
    // The registry directory resolves against the working directory, and
    // an absent one reads as a registry that declares nothing.
    if registry.is_empty() && !registry_dir.is_dir() {
        eprintln!(
            "warning: no title registry directory {} under the working directory; nothing \
             declares a title, so every installed title reads as an orphan and no cell is named",
            registry_dir.display()
        );
    }
    StoreView {
        root: root.to_path_buf(),
        inventory,
        registry,
        fixtures: crate::paths::fixtures_dir(&crate::paths::workspace_root()),
    }
}

/// Where the read commands look for the store, given `--vfs-root`.
pub(crate) fn store_root(vfs_flag: Option<&Path>) -> PathBuf {
    crate::cli::keys::install_root_of(&crate::cli::title::resolve_ps3_vfs_root(vfs_flag))
}
