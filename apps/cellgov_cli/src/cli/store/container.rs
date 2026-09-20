//! What `firmware install`, `title install`, and `self decrypt` share:
//! the container they map, the figures they print, and the operator's
//! vault.

use std::path::Path;

use cellgov_terminal::caps::{RenderFlags, TermCaps};

/// Terminal capabilities for a store install, capped at plain when the
/// firmware section trace is on.
///
/// The trace writes to stderr, so an in-place bar frame would cursor-up
/// over its lines.
pub(crate) fn install_caps(render: RenderFlags) -> TermCaps {
    cap_for_trace(render.caps(), cellgov_install::sce::section_trace_enabled())
}

fn cap_for_trace(caps: TermCaps, section_trace: bool) -> TermCaps {
    if section_trace {
        caps.capped_at_plain()
    } else {
        caps
    }
}

/// The container's filename, for the progress bar's title line.
pub(crate) fn container_label(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

/// Maps a container for a sequential read.
///
/// A disc image can exceed host RAM. Every installer reads its
/// container in sequence, so the host streams pages in and evicts them.
///
/// # Errors
///
/// Returns an error if the host cannot map `path`.
pub(crate) fn map_container(
    path: &Path,
) -> Result<filebuffer::FileBuffer, crate::cli::exit::CommandError> {
    filebuffer::FileBuffer::open(path).map_err(|error| {
        crate::cli::exit::CommandError::failed(format!("failed to map {}: {error}", path.display()))
    })
}

/// The megabyte figure the banner lines print.
pub(crate) fn megabytes(len: usize) -> f64 {
    len as f64 / (1024.0 * 1024.0)
}

pub(crate) fn vault(
    store: &Path,
) -> Result<cellgov_install::keys::KeyVault, crate::cli::exit::CommandError> {
    cellgov_install::keys::KeyVault::load_for_vfs(store)
        .map_err(|error| crate::cli::exit::CommandError::failed(error.to_string()))
}

#[cfg(test)]
#[path = "tests/container_tests.rs"]
mod tests;
