//! Where the store commands read the title registry.

use std::path::PathBuf;

/// Resolves from the compiled-in workspace root, so this path pairs
/// with the committed anchors under `tests/fixtures/`, which resolve
/// the same way.
pub(crate) fn registry_dir() -> PathBuf {
    crate::paths::workspace_root().join(crate::cli::title::DEFAULT_TITLE_REGISTRY_DIR)
}
