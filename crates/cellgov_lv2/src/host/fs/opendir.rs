//! `sys_fs_opendir` host dispatch.

use cellgov_event::UnitId;
use cellgov_ps3_abi::cell_errors as errno;

use crate::dispatch::Lv2Dispatch;
use crate::fs_store::FsError;
use crate::host::{Lv2Host, Lv2Runtime};

use super::mount::DirMountResolution;
use super::path::read_path_bytes;
use super::ptr::out_ptr_writable;

impl Lv2Host {
    /// `sys_fs_opendir` -- snapshot a host-mounted directory and
    /// allocate a directory fd over the lexicographically-sorted
    /// entries.
    ///
    /// # Error precedence
    ///
    /// In order:
    /// 1. `fd_out_ptr` NULL / misaligned / unwritable -> CELL_EFAULT,
    ///    no effects.
    /// 2. `path_ptr` unmapped, scan crosses unmapped, or no NUL
    ///    within `CELL_FS_MAX_PATH_LENGTH` -> CELL_EFAULT or CELL_EINVAL,
    ///    no effects.
    /// 3. Path is not a UTF-8 string -> CELL_ENOENT (no UTF-8 mount
    ///    can name a non-UTF-8 path).
    /// 4. Mount table has no matching prefix -> CELL_ENOENT.
    /// 5. Mount matches but host path is not a directory ->
    ///    CELL_ENOTDIR; missing -> CELL_ENOENT; IO error ->
    ///    CELL_EIO; `..` traversal -> CELL_EACCES.
    /// 6. Otherwise CELL_OK with one fd-write effect.
    pub(in crate::host) fn dispatch_fs_opendir(
        &mut self,
        path_ptr: u32,
        fd_out_ptr: u32,
        requester: UnitId,
        rt: &dyn Lv2Runtime,
    ) -> Lv2Dispatch {
        if !out_ptr_writable(rt, fd_out_ptr, 4, 4) {
            return Lv2Dispatch::Immediate {
                code: errno::CELL_EFAULT.into(),
                effects: vec![],
            };
        }

        let path_bytes_owned = match read_path_bytes(rt, path_ptr) {
            Ok(b) => b,
            Err(err) => {
                return Lv2Dispatch::Immediate {
                    code: err.into(),
                    effects: vec![],
                };
            }
        };

        let path = match std::str::from_utf8(&path_bytes_owned) {
            Ok(s) => s,
            Err(_) => {
                return Lv2Dispatch::Immediate {
                    code: errno::CELL_ENOENT.into(),
                    effects: vec![],
                };
            }
        };

        let path_owned = path.to_string();
        let entries = match self.try_mount_resolve_dir(&path_owned) {
            DirMountResolution::Snapshot(e) => e,
            DirMountResolution::Failed(err) => {
                return Lv2Dispatch::Immediate {
                    code: err.into(),
                    effects: vec![],
                };
            }
            DirMountResolution::Unmounted => {
                return Lv2Dispatch::Immediate {
                    code: errno::CELL_ENOENT.into(),
                    effects: vec![],
                };
            }
        };

        match self.fs_store_mut().open_dir(entries) {
            Ok(fd) => {
                self.fs_fd_count_inc();
                self.immediate_write_u32(fd, fd_out_ptr, requester)
            }
            Err(FsError::FdExhausted) => Lv2Dispatch::Immediate {
                code: errno::CELL_EMFILE.into(),
                effects: vec![],
            },
            Err(other) => {
                self.record_invariant_break(
                    "dispatch.fs_opendir.unexpected_fs_error",
                    format_args!(
                        "FsStore::open_dir returned {other:?} for {path_owned:?}; \
                         contract violated"
                    ),
                );
                Lv2Dispatch::Immediate {
                    code: errno::CELL_EFAULT.into(),
                    effects: vec![],
                }
            }
        }
    }
}
