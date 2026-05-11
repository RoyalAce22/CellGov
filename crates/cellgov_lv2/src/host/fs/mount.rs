//! Mount-table resolution with single-read disk caching.

use std::path::PathBuf;

use cellgov_ps3_abi::cell_errors as errno;

use crate::fs_store::{DirEntry, FsError};
use crate::host::Lv2Host;

/// Outcome of a host-side mount-table lookup for a regular-file
/// path. The dispatch layer uses this to choose between
/// re-querying the in-memory FS (because the bytes were just
/// cached), surfacing an errno (mount matched but the host file
/// was missing or unreadable), or falling through to legacy
/// behavior (no mount matched the guest path).
pub(super) enum MountResolution {
    /// No mount prefix matched. Caller should run its existing
    /// fallback path (whitelist, ENOENT, etc.).
    Unmounted,
    /// Bytes were read from the host file and registered as a blob
    /// under the original guest path. Caller should re-query the
    /// in-memory FS.
    Cached,
    /// Mount matched but the host-side lookup failed. Caller should
    /// surface `code` to the guest with no out-pointer write.
    Failed(cellgov_ps3_abi::cell_errors::Lv2Error),
}

/// Outcome of a host-side mount-table lookup for a directory
/// path. Directory enumeration does not pre-cache anything in
/// `FsStore`; each `sys_fs_opendir` snapshots the host directory
/// fresh and stores the entries on the directory fd.
pub(super) enum DirMountResolution {
    /// No mount prefix matched.
    Unmounted,
    /// Mount matched and the host directory enumerated cleanly.
    /// Entries are sorted lexicographically and filtered to
    /// regular files / directories (symlinks and special files
    /// dropped per anti-scope).
    Snapshot(Vec<DirEntry>),
    /// Mount matched but the host-side lookup failed (missing,
    /// not-a-directory, IO error). Caller surfaces `code` to the
    /// guest.
    Failed(cellgov_ps3_abi::cell_errors::Lv2Error),
}

impl Lv2Host {
    /// Try to satisfy a guest path via the mount table. On a
    /// successful resolve the host file is read once and cached in
    /// [`Self::fs_store`] so subsequent opens hit the in-memory
    /// blob (the determinism contract: single-read caching, no
    /// host time, content immutable thereafter).
    ///
    /// `path` is the UTF-8 guest path, already null-trimmed by the
    /// caller. The cache key is the same guest path so the next
    /// `open_fd` / `stat_path` for the same string hits the cache
    /// without re-resolving.
    pub(super) fn try_mount_resolve_and_cache(&mut self, path: &str) -> MountResolution {
        let host_path = match resolve_path(self, path) {
            Ok(p) => p,
            Err(MountResolveErr::Unmounted) => return MountResolution::Unmounted,
            Err(MountResolveErr::Failed(code)) => return MountResolution::Failed(code),
        };

        // is_file() rejects directories; directory consumers go
        // through [`Self::try_mount_resolve_dir`] instead.
        match std::fs::metadata(&host_path) {
            Ok(md) if md.is_file() => {}
            Ok(_) => return MountResolution::Failed(errno::CELL_ENOENT),
            Err(_) => return MountResolution::Failed(errno::CELL_ENOENT),
        }

        let bytes = match std::fs::read(&host_path) {
            Ok(b) => b,
            Err(_) => return MountResolution::Failed(errno::CELL_EIO),
        };

        match self.fs_store_mut().register_blob(path.to_string(), bytes) {
            Ok(()) => MountResolution::Cached,
            Err(FsError::PathAlreadyRegistered) => {
                // We only enter this branch after open_fd /
                // stat_path returned UnknownPath, so the blob was
                // not registered under this path moments ago.
                // Single-threaded LV2 dispatch means no race could
                // have populated it. Surface as an invariant break
                // rather than a silent success.
                self.record_invariant_break(
                    "dispatch.fs.mount_register_double",
                    format_args!(
                        "register_blob returned PathAlreadyRegistered for {path:?} \
                         after UnknownPath; contract violated"
                    ),
                );
                MountResolution::Failed(errno::CELL_EFAULT)
            }
            Err(other) => {
                self.record_invariant_break(
                    "dispatch.fs.mount_register_unexpected",
                    format_args!(
                        "register_blob returned {other:?} for {path:?}; contract violated"
                    ),
                );
                MountResolution::Failed(errno::CELL_EFAULT)
            }
        }
    }

    /// Try to satisfy a guest directory path via the mount table.
    /// Returns the snapshotted entries on success; the caller is
    /// expected to allocate a directory fd against them via
    /// [`crate::fs_store::FsStore::open_dir`].
    ///
    /// Determinism contract:
    /// - Entries sorted by `name` in lexicographic byte order.
    /// - Symlinks and special files are dropped (the oracle does
    ///   not surface symlink content; titles that need them are
    ///   anti-scope).
    /// - Non-UTF-8 host filenames are dropped (cannot be written
    ///   into the guest's UTF-8 / Shift-JIS path strings without
    ///   ambiguity); the determinism contract requires every
    ///   surfaced entry name be a stable, encoder-clean string.
    pub(super) fn try_mount_resolve_dir(&mut self, path: &str) -> DirMountResolution {
        let host_path = match resolve_path(self, path) {
            Ok(p) => p,
            Err(MountResolveErr::Unmounted) => return DirMountResolution::Unmounted,
            Err(MountResolveErr::Failed(code)) => return DirMountResolution::Failed(code),
        };

        match std::fs::metadata(&host_path) {
            Ok(md) if md.is_dir() => {}
            Ok(_) => return DirMountResolution::Failed(errno::CELL_ENOTDIR),
            Err(_) => return DirMountResolution::Failed(errno::CELL_ENOENT),
        }

        let read_dir = match std::fs::read_dir(&host_path) {
            Ok(rd) => rd,
            Err(_) => return DirMountResolution::Failed(errno::CELL_EIO),
        };

        let mut entries: Vec<DirEntry> = Vec::new();
        for entry in read_dir {
            let entry = match entry {
                Ok(e) => e,
                Err(_) => return DirMountResolution::Failed(errno::CELL_EIO),
            };
            // file_type() does not follow symlinks; metadata() would.
            let file_type = match entry.file_type() {
                Ok(ft) => ft,
                Err(_) => return DirMountResolution::Failed(errno::CELL_EIO),
            };
            // Anti-scope: symlinks and special files are not
            // enumerated. The oracle surfaces only regular files
            // and directories.
            let is_directory = if file_type.is_dir() {
                true
            } else if file_type.is_file() {
                false
            } else {
                continue;
            };
            // Non-UTF-8 host names cannot be re-encoded into the
            // guest's path string deterministically; drop them
            // rather than guess an encoding.
            let name = match entry.file_name().into_string() {
                Ok(s) => s,
                Err(_) => continue,
            };
            entries.push(DirEntry { name, is_directory });
        }
        // Lexicographic byte order on the name. read_dir yields
        // entries in raw filesystem order which is host-FS-specific
        // and a determinism vector; sorting collapses that.
        entries.sort_by(|a, b| a.name.as_bytes().cmp(b.name.as_bytes()));
        DirMountResolution::Snapshot(entries)
    }
}

/// Inner error of `resolve_path`: either no mount matched or the
/// resolve itself failed (path traversal, internal contract drift).
enum MountResolveErr {
    Unmounted,
    Failed(cellgov_ps3_abi::cell_errors::Lv2Error),
}

/// Shared prefix-resolution step for the file and directory
/// surfaces, including `..` rejection and invariant-break wiring.
fn resolve_path(host: &mut Lv2Host, path: &str) -> Result<PathBuf, MountResolveErr> {
    match host.fs_mounts().resolve(path) {
        Ok(Some(p)) => Ok(p),
        Ok(None) => Err(MountResolveErr::Unmounted),
        Err(FsError::PathTraversal) => Err(MountResolveErr::Failed(errno::CELL_EACCES)),
        Err(other) => {
            host.record_invariant_break(
                "dispatch.fs.mount_resolve_unexpected",
                format_args!(
                    "FsMountTable::resolve returned {other:?} for {path:?}; contract violated"
                ),
            );
            Err(MountResolveErr::Failed(errno::CELL_EFAULT))
        }
    }
}
