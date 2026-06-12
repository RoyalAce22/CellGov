//! `sys_fs_opendir` host dispatch.

use cellgov_event::UnitId;
use cellgov_ps3_abi::cell_errors;

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
    /// # Errors
    ///
    /// In precedence order:
    /// 1. `fd_out_ptr` NULL / misaligned / unwritable -> CELL_EFAULT.
    /// 2. `path_ptr` unmapped / no NUL within `CELL_FS_MAX_PATH_LENGTH`
    ///    -> CELL_EFAULT or CELL_EINVAL.
    /// 3. Non-UTF-8 path -> CELL_ENOENT.
    /// 4. No matching mount prefix -> CELL_ENOENT.
    /// 5. Host path not-a-directory -> CELL_ENOTDIR; missing ->
    ///    CELL_ENOENT; IO error -> CELL_EIO; `..` traversal ->
    ///    CELL_EACCES.
    /// 6. Otherwise CELL_OK with one fd-write effect.
    pub(in crate::host) fn dispatch_fs_opendir(
        &mut self,
        path_ptr: u32,
        fd_out_ptr: u32,
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

        let path = match std::str::from_utf8(&path_bytes_owned) {
            Ok(s) => s,
            Err(_) => {
                return Lv2Dispatch::immediate(cell_errors::CELL_ENOENT.into());
            }
        };

        let path_owned = path.to_string();
        let entries = match self.try_mount_resolve_dir(&path_owned) {
            DirMountResolution::Snapshot(e) => e,
            DirMountResolution::Failed(err) => {
                return Lv2Dispatch::immediate(err.into());
            }
            DirMountResolution::Unmounted => {
                return Lv2Dispatch::immediate(cell_errors::CELL_ENOENT.into());
            }
        };

        match self.fs_store_mut().open_dir(entries) {
            Ok(fd) => {
                self.fs_fd_count_inc();
                self.immediate_write_u32(fd, fd_out_ptr, requester)
            }
            Err(FsError::FdExhausted) => Lv2Dispatch::immediate(cell_errors::CELL_EMFILE.into()),
            Err(other) => {
                self.record_invariant_break(
                    "dispatch.fs_opendir.unexpected_fs_error",
                    format_args!(
                        "FsStore::open_dir returned {other:?} for {path_owned:?}; \
                         contract violated"
                    ),
                );
                Lv2Dispatch::immediate(cell_errors::CELL_EFAULT.into())
            }
        }
    }
}

#[cfg(test)]
#[path = "tests/opendir_tests.rs"]
mod tests;
