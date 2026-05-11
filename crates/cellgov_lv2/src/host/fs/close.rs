//! `sys_fs_close` host dispatch.

use cellgov_ps3_abi::cell_errors as errno;

use crate::dispatch::Lv2Dispatch;
use crate::fs_store::FsError;
use crate::host::Lv2Host;

impl Lv2Host {
    /// `sys_fs_close` -- release an fd allocated via the FS layer.
    ///
    /// FsStore-tracked fds are removed from the open-fd table so
    /// subsequent reads / fstats / closes via the FS layer see them
    /// as unknown (CELL_EBADF, the spec-correct outcome). After
    /// whitelist retirement, every fd above `FD_BASE` is FsStore-
    /// allocated, so a close on a never-allocated fd is a genuine
    /// guest bug -- surface as EBADF rather than silently CELL_OK.
    ///
    /// `fs_fd_count` is not decremented either way: real PS3 keeps
    /// the kernel-side fs-object count untouched across
    /// `sys_fs_close`, and the `sys_process_get_number_of_object`
    /// matrix in ps3autotests pins this.
    pub(in crate::host) fn dispatch_fs_close(&mut self, fd: u32) -> Lv2Dispatch {
        match self.fs_store_mut().close_fd(fd) {
            Ok(()) => Lv2Dispatch::Immediate {
                code: 0,
                effects: vec![],
            },
            Err(FsError::UnknownFd) => Lv2Dispatch::Immediate {
                code: errno::CELL_EBADF.into(),
                effects: vec![],
            },
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
                Lv2Dispatch::Immediate {
                    code: errno::CELL_EFAULT.into(),
                    effects: vec![],
                }
            }
        }
    }
}
