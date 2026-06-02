//! `sys_fs_open` host dispatch.

use cellgov_event::UnitId;
use cellgov_ps3_abi::cell_errors;

use crate::dispatch::Lv2Dispatch;
use crate::fs_store::FsError;
use crate::host::{Lv2Host, Lv2Runtime};

use super::flags::validate_open_flags;
use super::mount::MountResolution;
use super::path::read_path_bytes;
use super::ptr::out_ptr_writable;

impl Lv2Host {
    /// `sys_fs_open` -- allocate a read-only fd against either a
    /// pre-registered manifest blob or a path resolved through the
    /// mount table.
    ///
    /// # Errors
    ///
    /// In precedence order:
    /// 1. `fd_out_ptr` NULL / misaligned / unwritable -> CELL_EFAULT.
    /// 2. `path_ptr` unmapped / no NUL within `CELL_FS_MAX_PATH_LENGTH`
    ///    -> CELL_EFAULT or CELL_EINVAL.
    /// 3. Path exists AND flags request write semantics -> CELL_EROFS.
    /// 4. Path exists, flags OK -> CELL_OK with one fd-write effect.
    /// 5. Path missing or non-UTF-8 -> CELL_ENOENT.
    pub(in crate::host) fn dispatch_fs_open(
        &mut self,
        path_ptr: u32,
        flags: u32,
        fd_out_ptr: u32,
        _mode: u32,
        requester: UnitId,
        rt: &dyn Lv2Runtime,
    ) -> Lv2Dispatch {
        if !out_ptr_writable(rt, fd_out_ptr, 4, 4) {
            return Lv2Dispatch::immediate(cell_errors::CELL_EFAULT.into());
        }

        let path_bytes_owned = match read_path_bytes(rt, path_ptr) {
            Ok(b) => b,
            Err(err) => {
                return Lv2Dispatch::immediate(err.into());
            }
        };

        let Ok(p) = std::str::from_utf8(&path_bytes_owned) else {
            return Lv2Dispatch::immediate(cell_errors::CELL_ENOENT.into());
        };

        let flag_err = validate_open_flags(flags, p);

        // Existence-then-flag: a write-flag error wins only when the
        // path exists. Probe manifest before mount-table cache.
        if self.fs_store().has_path(p) {
            if let Some(err) = flag_err {
                return Lv2Dispatch::immediate(err.into());
            }
            return self.open_existing_blob(p, fd_out_ptr, requester);
        }

        match self.try_mount_resolve_and_cache(p) {
            MountResolution::Cached => {
                if let Some(err) = flag_err {
                    return Lv2Dispatch::immediate(err.into());
                }
                self.open_existing_blob(p, fd_out_ptr, requester)
            }
            MountResolution::Failed(err) => Lv2Dispatch::immediate(err.into()),
            MountResolution::Unmounted => Lv2Dispatch::immediate(cell_errors::CELL_ENOENT.into()),
        }
    }

    /// Allocate an fd against a path that was just confirmed to
    /// exist in [`Self::fs_store`].
    ///
    /// Caller invariant: `has_path(path)` returned true, so
    /// `open_fd` may return only `Ok(fd)` or `FdExhausted`; any
    /// other variant is internal-state drift and surfaces as an
    /// invariant break + EFAULT.
    fn open_existing_blob(
        &mut self,
        path: &str,
        fd_out_ptr: u32,
        requester: UnitId,
    ) -> Lv2Dispatch {
        match self.fs_store_mut().open_fd(path) {
            Ok(fd) => {
                self.fs_fd_count_inc();
                self.immediate_write_u32(fd, fd_out_ptr, requester)
            }
            Err(FsError::FdExhausted) => Lv2Dispatch::immediate(cell_errors::CELL_EMFILE.into()),
            Err(other) => {
                self.record_invariant_break(
                    "dispatch.fs_open.path_table_vs_fd_allocator_drift",
                    format_args!(
                        "FsStore::open_fd returned {other:?} for {path:?} \
                         after has_path was true; the path table and \
                         fd allocator disagree about the same path"
                    ),
                );
                Lv2Dispatch::immediate(cell_errors::CELL_EFAULT.into())
            }
        }
    }
}
