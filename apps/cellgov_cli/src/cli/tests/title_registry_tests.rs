//! The directory `--title` and `--content-id` resolve against.

use std::path::Path;

use cellgov_install::store::{StoreLayout, DEFAULT_VFS_ROOT};

use super::DEFAULT_TITLE_REGISTRY_DIR;
use cellgov_boot::manifest::TitleRegistry;

#[test]
fn the_default_registry_directory_holds_the_committed_manifests() {
    // `scan_dir` answers an absent directory with an empty registry. A
    // constant that names the wrong directory therefore reports every
    // title as unknown, and never names the path it missed.
    let dir = crate::paths::workspace_root().join(DEFAULT_TITLE_REGISTRY_DIR);
    let registry =
        TitleRegistry::scan_dir(&dir).unwrap_or_else(|e| panic!("scan {}: {e}", dir.display()));
    assert!(
        registry.iter().count() > 0,
        "{} holds no manifest",
        dir.display()
    );
}

#[test]
fn the_registry_directory_is_not_the_store_content_directory() {
    assert_ne!(
        Path::new(DEFAULT_VFS_ROOT).join(DEFAULT_TITLE_REGISTRY_DIR),
        StoreLayout::new(DEFAULT_VFS_ROOT).titles_root(),
        "the manifest registry and the store's installed-title content \
         resolve to one directory; one of the two names has to change"
    );
}
