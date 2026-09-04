//! What `firmware install`, `title install`, and `self decrypt` share:
//! the container they map, the figures they print, and the operator's
//! vault.

use std::path::Path;

/// The container's filename, for the progress bar's title line.
pub(crate) fn container_label(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

/// Map a container for a sequential read.
///
/// A disc image can exceed host RAM. Every installer reads its
/// container in sequence, so the host streams pages in and evicts them.
pub(crate) fn map_container_or_die(path: &Path) -> filebuffer::FileBuffer {
    filebuffer::FileBuffer::open(path).unwrap_or_else(|e| {
        crate::cli::exit::die(&format!("failed to map {}: {e}", path.display()))
    })
}

/// The megabyte figure the banner lines print.
pub(crate) fn megabytes(len: usize) -> f64 {
    len as f64 / (1024.0 * 1024.0)
}

/// Load the operator's vault for `store`, or die naming the refusal.
pub(crate) fn vault_or_die(store: &Path) -> cellgov_install::keys::KeyVault {
    cellgov_install::keys::KeyVault::load_for_vfs(store)
        .unwrap_or_else(|e| crate::cli::exit::die(&e.to_string()))
}
