//! Error enum surfaced by the FS layer. Dispatch handlers map these
//! to CELL_FS_* errno values.

/// Errors the FS layer surfaces. Dispatch handlers map these to
/// CELL_FS_* errno values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum FsError {
    /// Fd is not in the open-fd table.
    #[error("unknown file descriptor")]
    UnknownFd,
    /// Distinct from `UnknownFd` so `close_fd` on a dir fd (or
    /// `close_dir` on a file fd) surfaces as CELL_EBADF rather than
    /// silent success.
    #[error("unknown directory descriptor")]
    UnknownDir,
    /// Path is not registered in the blob table.
    #[error("unknown path")]
    UnknownPath,
    /// Seek offset would land outside `[0, u64::MAX]`.
    #[error("seek offset out of range")]
    SeekOutOfRange,
    /// Fd allocator hit `u32::MAX`. Surfaced rather than recycled
    /// so an aliased read cannot advance another open's offset.
    #[error("fd allocator exhausted")]
    FdExhausted,
    /// Registration is single-write per path so a later content swap
    /// cannot mutate bytes an open fd is reading from.
    #[error("path already registered")]
    PathAlreadyRegistered,
    /// Single-write per prefix to keep guest-to-host resolution
    /// unambiguous.
    #[error("mount prefix already registered")]
    MountAlreadyRegistered,
    /// Surfaced as CELL_EACCES by the dispatch layer.
    #[error("path traversal rejected")]
    PathTraversal,
}
