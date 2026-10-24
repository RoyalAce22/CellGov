//! Removals for which an absent target is already the wanted end state.

use std::io;
use std::path::Path;

/// Remove the file at `path`; `false` when it was already absent.
///
/// # Errors
///
/// Every removal failure except absence.
pub(crate) fn remove_file_if_present(path: &Path) -> io::Result<bool> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(true),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e),
    }
}

/// Remove the directory tree at `path`; `false` when it was already
/// absent.
///
/// # Errors
///
/// Every removal failure except absence.
pub(crate) fn remove_dir_all_if_present(path: &Path) -> io::Result<bool> {
    match std::fs::remove_dir_all(path) {
        Ok(()) => Ok(true),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e),
    }
}
