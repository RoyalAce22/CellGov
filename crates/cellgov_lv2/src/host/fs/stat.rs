//! `sys_fs_fstat` and `sys_fs_stat` host dispatch.

use cellgov_event::UnitId;
use cellgov_ps3_abi::cell_errors as errno;

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
    /// # Error precedence
    ///
    /// 1. `stat_out_ptr` misaligned / unwritable for 56 bytes ->
    ///    CELL_EFAULT, no effects.
    /// 2. Unknown `fd` -> CELL_EBADF, no effects.
    /// 3. Otherwise CELL_OK with a single 56-byte struct write.
    pub(in crate::host) fn dispatch_fs_fstat(
        &mut self,
        fd: u32,
        stat_out_ptr: u32,
        requester: UnitId,
        rt: &dyn Lv2Runtime,
    ) -> Lv2Dispatch {
        if !is_stat_ptr_writable(rt, stat_out_ptr) {
            return Lv2Dispatch::Immediate {
                code: errno::CELL_EFAULT.into(),
                effects: vec![],
            };
        }
        let stat = match self.fs_store().fstat(fd) {
            Ok(s) => s,
            Err(FsError::UnknownFd) => {
                return Lv2Dispatch::Immediate {
                    code: errno::CELL_EBADF.into(),
                    effects: vec![],
                };
            }
            Err(other) => {
                self.record_invariant_break(
                    "dispatch.fs_fstat.unexpected_fs_error",
                    format_args!(
                        "FsStore::fstat returned {other:?} for fd={fd:#x}; \
                         contract violated"
                    ),
                );
                return Lv2Dispatch::Immediate {
                    code: errno::CELL_EFAULT.into(),
                    effects: vec![],
                };
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
    /// # Error precedence
    ///
    /// 1. `stat_out_ptr` misaligned / unwritable for 56 bytes ->
    ///    CELL_EFAULT, no effects.
    /// 2. `path_ptr` unmapped or no NUL within `CELL_FS_MAX_PATH_LENGTH` ->
    ///    CELL_EFAULT or CELL_EINVAL, no effects (mirrors
    ///    `dispatch_fs_open`).
    /// 3. Path not registered in the FS layer -> CELL_ENOENT, no
    ///    effects.
    /// 4. Otherwise CELL_OK with a single 56-byte struct write.
    pub(in crate::host) fn dispatch_fs_stat(
        &mut self,
        path_ptr: u32,
        stat_out_ptr: u32,
        requester: UnitId,
        rt: &dyn Lv2Runtime,
    ) -> Lv2Dispatch {
        if !is_stat_ptr_writable(rt, stat_out_ptr) {
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
        // Non-UTF-8 paths can never match a manifest blob (manifest
        // keys are UTF-8); short-circuit to ENOENT before touching
        // FsStore.
        let path_str = match std::str::from_utf8(&path_bytes_owned) {
            Ok(s) => s,
            Err(_) => {
                return Lv2Dispatch::Immediate {
                    code: errno::CELL_ENOENT.into(),
                    effects: vec![],
                };
            }
        };
        let stat = match self.fs_store().stat_path(path_str) {
            Ok(s) => s,
            Err(FsError::UnknownPath) => {
                // Owned to release the &self borrow before we
                // touch &mut self via try_mount_resolve_and_cache.
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
                            return Lv2Dispatch::Immediate {
                                code: errno::CELL_EFAULT.into(),
                                effects: vec![],
                            };
                        }
                    },
                    MountResolution::Failed(err) => {
                        return Lv2Dispatch::Immediate {
                            code: err.into(),
                            effects: vec![],
                        };
                    }
                    MountResolution::Unmounted => {
                        return Lv2Dispatch::Immediate {
                            code: errno::CELL_ENOENT.into(),
                            effects: vec![],
                        };
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
                return Lv2Dispatch::Immediate {
                    code: errno::CELL_EFAULT.into(),
                    effects: vec![],
                };
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
