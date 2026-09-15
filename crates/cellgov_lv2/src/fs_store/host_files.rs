//! The file source a mount table reads its host roots through.

use std::ffi::OsString;
use std::io::ErrorKind;
use std::path::Path;

/// What a host path names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostEntryKind {
    /// A regular file.
    File,
    /// A directory.
    Directory,
    /// Any other kind of entry:
    ///
    /// - a special file.
    /// - a symlink that [`MountFiles::list`] does not follow.
    Other,
}

/// One name a host directory lists.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostDirEntry {
    /// The name as the host spells it.
    pub name: OsString,
    /// What the name is, without following a symlink.
    pub kind: HostEntryKind,
}

/// How a mount table reads its host roots.
///
/// The program driving the runtime supplies the file source through
/// [`super::FsMountTable::set_files`]. The mount layer maps each refused
/// [`ErrorKind`] to a guest errno.
///
/// A successful `read` runs once per guest path per boot, because the
/// mount layer caches its bytes. Every other lookup calls the file
/// source again.
pub trait MountFiles: std::fmt::Debug {
    /// What `path` names, following a symlink.
    ///
    /// # Errors
    ///
    /// The host's refusal to describe `path`. The mount layer reads
    /// these kinds as "nothing at `path`" and tries the next root:
    ///
    /// - `NotFound`
    /// - `NotADirectory`
    /// - `InvalidFilename`
    ///
    /// Every other kind stops the lookup at this root.
    fn kind(&self, path: &Path) -> Result<HostEntryKind, ErrorKind>;

    /// Every byte of the file at `path`.
    ///
    /// # Errors
    ///
    /// The host's refusal to read `path`.
    fn read(&self, path: &Path) -> Result<Vec<u8>, ErrorKind>;

    /// Every name directly under the directory `path`, in host order.
    ///
    /// # Errors
    ///
    /// The host's refusal to list `path` or to describe one of its
    /// names.
    fn list(&self, path: &Path) -> Result<Vec<HostDirEntry>, ErrorKind>;
}

/// The null backend: refuses every call with [`ErrorKind::Unsupported`].
///
/// On this backend, each mount lookup that reaches the file source
/// answers the guest with `CELL_EIO` and logs a named invariant break.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoMountFiles;

impl MountFiles for NoMountFiles {
    fn kind(&self, _path: &Path) -> Result<HostEntryKind, ErrorKind> {
        Err(ErrorKind::Unsupported)
    }

    fn read(&self, _path: &Path) -> Result<Vec<u8>, ErrorKind> {
        Err(ErrorKind::Unsupported)
    }

    fn list(&self, _path: &Path) -> Result<Vec<HostDirEntry>, ErrorKind> {
        Err(ErrorKind::Unsupported)
    }
}
