//! `sys_fs_opendir` host dispatch.

use cellgov_event::UnitId;
use cellgov_ps3_abi::lv2::errno;

use crate::dispatch::Lv2Dispatch;
use crate::fs_store::{DirEntry, FsError};
use crate::host::{Lv2Host, Lv2Runtime};

use super::mount::DirMountResolution;
use super::path::{read_path_bytes, unmounted_answer};
use super::ptr::out_ptr_writable;
use cellgov_time::GuestTicks;

impl Lv2Host {
    /// `sys_fs_opendir` -- snapshot a host-mounted directory and
    /// allocate a directory fd over its entries. Directories come from
    /// the mount table alone, so a manifest path never names one. Every
    /// root that holds the directory contributes to one listing. A name
    /// several roots hold takes its entry from the earliest root.
    ///
    /// The listing starts with `.` and `..`, both directories, as the
    /// `lv2_fs_answers` capture reads them. The console then lists the
    /// other entries in the order they were created; a host directory
    /// keeps no such order, so CellGov lists them in lexicographic
    /// byte order.
    ///
    /// # Errors
    ///
    /// In precedence order:
    /// 1. `fd_out_ptr` NULL / misaligned / unwritable -> CELL_EFAULT.
    /// 2. `path_ptr` unmapped -> CELL_EFAULT; a path or a name in it
    ///    over the limit -> CELL_ENAMETOOLONG.
    /// 3. Non-UTF-8 path -> CELL_ENOENT.
    /// 4. No matching mount prefix -> CELL_ENOTMOUNTED, or CELL_ENOENT
    ///    when the root is one a game process finds mounted
    ///    (`unmounted_answer`).
    /// 5. Host path not-a-directory -> CELL_ENOTDIR; missing ->
    ///    CELL_ENOENT; IO error -> CELL_EIO; `..` traversal ->
    ///    CELL_EACCES.
    /// 6. Otherwise CELL_OK with one fd-write effect, or CELL_EMFILE
    ///    when the fd allocator is exhausted.
    pub(in crate::host) fn dispatch_fs_opendir(
        &mut self,
        path_ptr: u32,
        fd_out_ptr: u32,
        requester: UnitId,
        rt: &dyn Lv2Runtime,
        tick: GuestTicks,
    ) -> Lv2Dispatch {
        if !out_ptr_writable(rt, fd_out_ptr, 4, 4) {
            return Lv2Dispatch::immediate(errno::CELL_EFAULT.into());
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
                return Lv2Dispatch::immediate(errno::CELL_ENOENT.into());
            }
        };

        let path_owned = path.to_string();
        let entries = match self.try_mount_resolve_dir(&path_owned) {
            DirMountResolution::Snapshot(e) => e,
            DirMountResolution::Failed(err) => {
                return Lv2Dispatch::immediate(err.into());
            }
            DirMountResolution::Unmounted => {
                return Lv2Dispatch::immediate(unmounted_answer(&path_owned).into());
            }
        };

        let listing = [".", ".."]
            .into_iter()
            .map(|name| DirEntry {
                name: name.to_string(),
                is_directory: true,
            })
            .chain(entries)
            .collect();
        match self.fs_store_mut().open_dir(listing) {
            Ok(fd) => {
                self.fs_fd_count_inc();
                self.immediate_write_u32(fd, fd_out_ptr, requester, tick)
            }
            Err(FsError::FdExhausted) => Lv2Dispatch::immediate(errno::CELL_EMFILE.into()),
            Err(other) => {
                self.record_invariant_break(
                    "dispatch.fs_opendir.unexpected_fs_error",
                    format_args!(
                        "FsStore::open_dir returned {other:?} for {path_owned:?}; \
                         contract violated"
                    ),
                );
                Lv2Dispatch::immediate(errno::CELL_EFAULT.into())
            }
        }
    }
}

#[cfg(test)]
#[path = "tests/opendir_tests.rs"]
mod tests;
