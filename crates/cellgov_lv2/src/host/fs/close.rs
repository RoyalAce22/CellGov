//! `sys_fs_close` host dispatch.

use cellgov_ps3_abi::lv2::errno;

use crate::dispatch::Lv2Dispatch;
use crate::fs_store::FsError;
use crate::host::Lv2Host;

impl Lv2Host {
    /// `sys_fs_close` -- release an fd allocated via the FS layer.
    /// Later reads, seeks and fstats on the fd answer CELL_EBADF.
    ///
    /// `fs_fd_count` stays unchanged: real PS3 keeps the kernel-side
    /// fs-object count untouched across `sys_fs_close`, and the
    /// hardware trace in `tests/ps3autotests/tests/lv2/sys_process`
    /// pins the count across a close.
    ///
    /// # Errors
    ///
    /// CELL_EBADF for an fd the open-file table does not hold. A
    /// directory fd lives in a distinct store, so it answers CELL_EBADF
    /// here too.
    pub(in crate::host) fn dispatch_fs_close(&mut self, fd: u32) -> Lv2Dispatch {
        match self.fs_store_mut().close_fd(fd) {
            Ok(()) => Lv2Dispatch::immediate(0),
            Err(FsError::UnknownFd) => Lv2Dispatch::immediate(errno::CELL_EBADF.into()),
            Err(other) => {
                self.record_invariant_break(
                    "dispatch.fs_close.unexpected_fs_error",
                    format_args!(
                        "FsStore::close_fd returned {other:?} for fd={fd:#x}; \
                         contract violated"
                    ),
                );
                Lv2Dispatch::immediate(errno::CELL_EFAULT.into())
            }
        }
    }
}

#[cfg(test)]
#[path = "tests/close_tests.rs"]
mod tests;
