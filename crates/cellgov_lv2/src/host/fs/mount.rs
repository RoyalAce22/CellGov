//! Mount-table resolution with single-read disk caching.

use std::path::PathBuf;

use cellgov_ps3_abi::cell_errors;

use crate::fs_store::{DirEntry, FsError};
use crate::host::Lv2Host;

/// Outcome of a host-side mount-table lookup for a regular-file path.
pub(super) enum MountResolution {
    /// No mount prefix matched.
    Unmounted,
    /// Bytes were read from the host file and registered as a blob
    /// under the original guest path; caller should re-query the
    /// in-memory FS.
    Cached,
    /// Mount matched but the host-side lookup failed.
    Failed(cellgov_ps3_abi::cell_errors::Lv2ErrCode),
}

/// Outcome of a host-side mount-table lookup for a directory path.
///
/// Each `sys_fs_opendir` snapshots the host directory fresh; nothing
/// is pre-cached in `FsStore`.
pub(super) enum DirMountResolution {
    Unmounted,
    /// Entries sorted lexicographically; symlinks, special files, and
    /// non-UTF-8 names dropped.
    Snapshot(Vec<DirEntry>),
    Failed(cellgov_ps3_abi::cell_errors::Lv2ErrCode),
}

impl Lv2Host {
    /// Try to satisfy a guest path via the mount table, caching the
    /// host file's bytes as a blob keyed on the guest path.
    ///
    /// Determinism contract: a single host read per guest path; the
    /// cached content is immutable thereafter.
    pub(super) fn try_mount_resolve_and_cache(&mut self, path: &str) -> MountResolution {
        let host_path = match resolve_path(self, path) {
            Ok(p) => p,
            Err(MountResolveErr::Unmounted) => return MountResolution::Unmounted,
            Err(MountResolveErr::Failed(code)) => return MountResolution::Failed(code),
        };

        match std::fs::metadata(&host_path) {
            Ok(md) if md.is_file() => {}
            Ok(_) => return MountResolution::Failed(cell_errors::CELL_ENOENT),
            Err(_) => return MountResolution::Failed(cell_errors::CELL_ENOENT),
        }

        let bytes = match std::fs::read(&host_path) {
            Ok(b) => b,
            Err(_) => return MountResolution::Failed(cell_errors::CELL_EIO),
        };

        match self.fs_store_mut().register_blob(path.to_string(), bytes) {
            Ok(()) => MountResolution::Cached,
            Err(FsError::PathAlreadyRegistered) => {
                self.record_invariant_break(
                    "dispatch.fs.mount_register_double",
                    format_args!(
                        "register_blob returned PathAlreadyRegistered for {path:?} \
                         after UnknownPath; contract violated"
                    ),
                );
                MountResolution::Failed(cell_errors::CELL_EFAULT)
            }
            Err(other) => {
                self.record_invariant_break(
                    "dispatch.fs.mount_register_unexpected",
                    format_args!(
                        "register_blob returned {other:?} for {path:?}; contract violated"
                    ),
                );
                MountResolution::Failed(cell_errors::CELL_EFAULT)
            }
        }
    }

    /// Try to satisfy a guest directory path via the mount table.
    ///
    /// Determinism contract:
    /// - Entries sorted by `name` in lexicographic byte order.
    /// - Symlinks, special files, and non-UTF-8 names are dropped.
    pub(super) fn try_mount_resolve_dir(&mut self, path: &str) -> DirMountResolution {
        let host_path = match resolve_path(self, path) {
            Ok(p) => p,
            Err(MountResolveErr::Unmounted) => return DirMountResolution::Unmounted,
            Err(MountResolveErr::Failed(code)) => return DirMountResolution::Failed(code),
        };

        match std::fs::metadata(&host_path) {
            Ok(md) if md.is_dir() => {}
            Ok(_) => return DirMountResolution::Failed(cell_errors::CELL_ENOTDIR),
            Err(_) => return DirMountResolution::Failed(cell_errors::CELL_ENOENT),
        }

        let read_dir = match std::fs::read_dir(&host_path) {
            Ok(rd) => rd,
            Err(_) => return DirMountResolution::Failed(cell_errors::CELL_EIO),
        };

        let mut entries: Vec<DirEntry> = Vec::new();
        for entry in read_dir {
            let entry = match entry {
                Ok(e) => e,
                Err(_) => return DirMountResolution::Failed(cell_errors::CELL_EIO),
            };
            let file_type = match entry.file_type() {
                Ok(ft) => ft,
                Err(_) => return DirMountResolution::Failed(cell_errors::CELL_EIO),
            };
            let is_directory = if file_type.is_dir() {
                true
            } else if file_type.is_file() {
                false
            } else {
                continue;
            };
            let name = match entry.file_name().into_string() {
                Ok(s) => s,
                Err(_) => continue,
            };
            entries.push(DirEntry { name, is_directory });
        }
        // Determinism: read_dir yields host-FS-specific order; sort
        // collapses that to lexicographic byte order.
        entries.sort_by(|a, b| a.name.as_bytes().cmp(b.name.as_bytes()));
        DirMountResolution::Snapshot(entries)
    }
}

#[derive(Debug, thiserror::Error)]
enum MountResolveErr {
    #[error("no mount matched")]
    Unmounted,
    #[error("mount resolve failed: lv2 errno 0x{:08x}", .0.code)]
    Failed(cellgov_ps3_abi::cell_errors::Lv2ErrCode),
}

/// Shared prefix-resolution step for the file and directory surfaces.
fn resolve_path(host: &mut Lv2Host, path: &str) -> Result<PathBuf, MountResolveErr> {
    match host.fs_mounts().resolve(path) {
        Ok(Some(p)) => Ok(p),
        Ok(None) => Err(MountResolveErr::Unmounted),
        Err(FsError::PathTraversal) => Err(MountResolveErr::Failed(cell_errors::CELL_EACCES)),
        Err(other) => {
            host.record_invariant_break(
                "dispatch.fs.mount_resolve_unexpected",
                format_args!(
                    "FsMountTable::resolve returned {other:?} for {path:?}; contract violated"
                ),
            );
            Err(MountResolveErr::Failed(cell_errors::CELL_EFAULT))
        }
    }
}
