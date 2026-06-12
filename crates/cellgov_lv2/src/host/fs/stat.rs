//! `sys_fs_fstat` and `sys_fs_stat` host dispatch.

use cellgov_event::UnitId;
use cellgov_ps3_abi::cell_errors;

use crate::dispatch::Lv2Dispatch;
use crate::fs_store::FsError;
use crate::host::{Lv2Host, Lv2Runtime};

use super::mount::MountResolution;
use super::path::read_path_bytes;
use super::stat_layout::{cell_fs_stat_write, is_stat_ptr_writable};

impl Lv2Host {
    /// `sys_fs_fstat` -- populate a `CellFsStat` (56 bytes) for an
    /// open fd's backing blob.
    ///
    /// # Errors
    ///
    /// In precedence order:
    /// 1. `stat_out_ptr` misaligned / unwritable for 56 bytes -> CELL_EFAULT.
    /// 2. Unknown `fd` -> CELL_EBADF.
    pub(in crate::host) fn dispatch_fs_fstat(
        &mut self,
        fd: u32,
        stat_out_ptr: u32,
        requester: UnitId,
        rt: &dyn Lv2Runtime,
    ) -> Lv2Dispatch {
        if !is_stat_ptr_writable(rt, stat_out_ptr) {
            return Lv2Dispatch::immediate(cell_errors::CELL_EFAULT.into());
        }
        let stat = match self.fs_store().fstat(fd) {
            Ok(s) => s,
            Err(FsError::UnknownFd) => {
                return Lv2Dispatch::immediate(cell_errors::CELL_EBADF.into());
            }
            Err(other) => {
                self.record_invariant_break(
                    "dispatch.fs_fstat.unexpected_fs_error",
                    format_args!(
                        "FsStore::fstat returned {other:?} for fd={fd:#x}; \
                         contract violated"
                    ),
                );
                return Lv2Dispatch::immediate(cell_errors::CELL_EFAULT.into());
            }
        };
        Lv2Dispatch::Immediate {
            code: 0,
            effects: vec![cell_fs_stat_write(
                stat,
                stat_out_ptr,
                requester,
                rt.current_tick(),
            )],
        }
    }

    /// `sys_fs_stat` -- path-keyed variant of `sys_fs_fstat`.
    ///
    /// # Errors
    ///
    /// In precedence order:
    /// 1. `stat_out_ptr` misaligned / unwritable for 56 bytes -> CELL_EFAULT.
    /// 2. `path_ptr` unmapped or no NUL within `CELL_FS_MAX_PATH_LENGTH`
    ///    -> CELL_EFAULT or CELL_EINVAL.
    /// 3. Path not registered -> CELL_ENOENT.
    pub(in crate::host) fn dispatch_fs_stat(
        &mut self,
        path_ptr: u32,
        stat_out_ptr: u32,
        requester: UnitId,
        rt: &dyn Lv2Runtime,
    ) -> Lv2Dispatch {
        if !is_stat_ptr_writable(rt, stat_out_ptr) {
            return Lv2Dispatch::immediate(cell_errors::CELL_EFAULT.into());
        }
        let path_bytes_owned = match read_path_bytes(rt, path_ptr) {
            Ok(b) => b,
            Err(err) => {
                return Lv2Dispatch::immediate(err.into());
            }
        };
        let path_str = match std::str::from_utf8(&path_bytes_owned) {
            Ok(s) => s,
            Err(_) => {
                return Lv2Dispatch::immediate(cell_errors::CELL_ENOENT.into());
            }
        };
        let stat = match self.fs_store().stat_path(path_str) {
            Ok(s) => s,
            Err(FsError::UnknownPath) => {
                let path_owned = path_str.to_string();
                match self.try_mount_resolve_and_cache(&path_owned) {
                    MountResolution::Cached => match self.fs_store().stat_path(&path_owned) {
                        Ok(s) => s,
                        Err(other) => {
                            self.record_invariant_break(
                                "dispatch.fs_stat.post_cache_stat_failed",
                                format_args!(
                                    "stat_path returned {other:?} for {path_owned:?} \
                                     after caching; contract violated"
                                ),
                            );
                            return Lv2Dispatch::immediate(cell_errors::CELL_EFAULT.into());
                        }
                    },
                    MountResolution::Failed(err) => {
                        return Lv2Dispatch::immediate(err.into());
                    }
                    MountResolution::Unmounted => {
                        return Lv2Dispatch::immediate(cell_errors::CELL_ENOENT.into());
                    }
                }
            }
            Err(other) => {
                self.record_invariant_break(
                    "dispatch.fs_stat.unexpected_fs_error",
                    format_args!(
                        "FsStore::stat_path returned {other:?} for {path_str:?}; \
                         contract violated"
                    ),
                );
                return Lv2Dispatch::immediate(cell_errors::CELL_EFAULT.into());
            }
        };
        Lv2Dispatch::Immediate {
            code: 0,
            effects: vec![cell_fs_stat_write(
                stat,
                stat_out_ptr,
                requester,
                rt.current_tick(),
            )],
        }
    }
}

#[cfg(test)]
#[path = "tests/stat_tests.rs"]
mod tests;
