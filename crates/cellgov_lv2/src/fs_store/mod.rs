//! In-memory filesystem layer.
//!
//! [`FsStore`] is host-side data populated at boot from a per-title
//! content manifest and consumed by the `sys_fs_*` LV2 dispatch
//! handlers. It is the back end for guest reads from paths a title
//! expects to load (`/app_home/Data/.../*.xml`, configuration files,
//! etc.); cellgov has no real filesystem so the bytes come from the
//! title manifest's `[content]` block.
//!
//! A blob is registered once at boot and is immutable thereafter.
//! Each `sys_fs_open` allocates a fresh fd whose offset advances
//! independently per fd; two opens of the same path each see the
//! file from byte 0.
//!
//! All ids are deterministic: fds come from a monotonic counter that
//! never recycles within a boot.

mod mount;
mod store;

pub use mount::{FsMount, FsMountTable};
pub use store::{DirEntry, FileStat, FsStore, SeekWhence};

/// Errors the FS layer surfaces. Dispatch handlers map these to
/// CELL_FS_* errno values.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FsError {
    /// Fd is not in the open-fd table.
    UnknownFd,
    /// Distinct from `UnknownFd` so `close_fd` on a dir fd (or
    /// `close_dir` on a file fd) surfaces as CELL_EBADF rather than
    /// silent success.
    UnknownDir,
    /// Path is not registered in the blob table.
    UnknownPath,
    /// Seek offset would land outside `[0, u64::MAX]`.
    SeekOutOfRange,
    /// Fd allocator hit `u32::MAX`. Surfaced rather than recycled
    /// so an aliased read cannot advance another open's offset.
    FdExhausted,
    /// Registration is single-write per path so a later content swap
    /// cannot mutate bytes an open fd is reading from.
    PathAlreadyRegistered,
    /// Single-write per prefix to keep guest-to-host resolution
    /// unambiguous.
    MountAlreadyRegistered,
    /// Surfaced as CELL_EACCES by the dispatch layer.
    PathTraversal,
}

impl std::fmt::Display for FsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownFd => f.write_str("unknown file descriptor"),
            Self::UnknownDir => f.write_str("unknown directory descriptor"),
            Self::UnknownPath => f.write_str("unknown path"),
            Self::SeekOutOfRange => f.write_str("seek offset out of range"),
            Self::FdExhausted => f.write_str("fd allocator exhausted"),
            Self::PathAlreadyRegistered => f.write_str("path already registered"),
            Self::MountAlreadyRegistered => f.write_str("mount prefix already registered"),
            Self::PathTraversal => f.write_str("path traversal rejected"),
        }
    }
}

impl std::error::Error for FsError {}
