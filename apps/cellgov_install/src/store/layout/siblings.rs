//! The hidden staging and tombstone siblings of an entry directory.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// A directory that names no entry, so no hidden sibling can stand
/// beside it:
///
/// - the empty path, `.`, or `..`,
/// - a path that ends in `.` or `..`,
/// - a filesystem root,
/// - a bare Win32 drive or UNC prefix.
///
/// The sibling is a rename target and a `remove_dir_all` argument, so a
/// name derived from no entry would act on the process working
/// directory.
#[derive(Debug, thiserror::Error)]
#[error("{} has no final component, so it names no staging or tombstone sibling", dir.display())]
pub struct HiddenSiblingError {
    /// The directory the sibling was to stand beside.
    pub dir: PathBuf,
}

/// The staging sibling of `final_dir`: `<parent>/.staging-<name>`.
///
/// The name is a function of the target, so an install interrupted
/// mid-stage leaves residue the next install of that target sweeps by
/// name.
///
/// # Errors
///
/// [`HiddenSiblingError`] when `final_dir` names no entry.
pub fn staging_sibling(final_dir: &Path) -> Result<PathBuf, HiddenSiblingError> {
    hidden_sibling(final_dir, "staging")
}

/// Directory a firmware install stages into, under the firmware root.
///
/// Outside [`hidden_sibling`]'s `.staging-<name>` / `.uninstalling-<name>`
/// namespace, so it cannot alias the residue of any one version:
/// `.staging-fw` would be [`staging_sibling`] of a firmware entry keyed
/// `fw`, which [`VersionKey`](super::VersionKey) accepts.
pub(super) const FIRMWARE_STAGING_DIR: &str = ".firmware-staging";

/// Lock file for the staging directory every firmware install shares.
///
/// [`VersionKey`](super::VersionKey) rejects a leading dot, so no installed version claims
/// this name.
pub(super) const FIRMWARE_STAGING_LOCK: &str = ".staging.lock";

/// The tombstone sibling of `final_dir`: `<parent>/.uninstalling-<name>`.
///
/// # Errors
///
/// [`HiddenSiblingError`] when `final_dir` names no entry.
pub fn tombstone_sibling(final_dir: &Path) -> Result<PathBuf, HiddenSiblingError> {
    hidden_sibling(final_dir, "uninstalling")
}

/// `<parent>/.<prefix>-<final component>`.
///
/// The sibling name carries the entry name verbatim, so two names that
/// differ only outside UTF-8 get two siblings.
fn hidden_sibling(final_dir: &Path, prefix: &str) -> Result<PathBuf, HiddenSiblingError> {
    match (final_dir.parent(), final_dir.file_name()) {
        (Some(parent), Some(name)) => {
            let mut sibling = OsString::from(format!(".{prefix}-"));
            sibling.push(name);
            Ok(parent.join(sibling))
        }
        _ => Err(HiddenSiblingError {
            dir: final_dir.to_path_buf(),
        }),
    }
}
