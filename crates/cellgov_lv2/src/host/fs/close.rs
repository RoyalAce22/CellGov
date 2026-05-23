//! `sys_fs_close` host dispatch.

use cellgov_ps3_abi::cell_errors as errno;

use crate::dispatch::Lv2Dispatch;
use crate::fs_store::FsError;
use crate::host::Lv2Host;

impl Lv2Host {
    /// `sys_fs_close` -- release an fd allocated via the FS layer.
    /// Unknown fds surface CELL_EBADF. `fs_fd_count` is not
    /// decremented: real PS3 keeps the kernel-side fs-object count
    /// untouched across `sys_fs_close`.
    pub(in crate::host) fn dispatch_fs_close(&mut self, fd: u32) -> Lv2Dispatch {
        match self.fs_store_mut().close_fd(fd) {
            Ok(()) => Lv2Dispatch::immediate(0),
            Err(FsError::UnknownFd) => Lv2Dispatch::immediate(errno::CELL_EBADF.into()),
            Err(other) => {
                // close_fd's contract: only Ok or UnknownFd.
                // Anything else means FsError grew without dispatch
                // being updated. Surface as host-bug EFAULT rather
                // than silently degrading to CELL_OK.
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
