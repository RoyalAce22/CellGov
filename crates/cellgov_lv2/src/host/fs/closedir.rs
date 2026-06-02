//! `sys_fs_closedir` host dispatch.

use cellgov_ps3_abi::cell_errors;

use crate::dispatch::Lv2Dispatch;
use crate::fs_store::FsError;
use crate::host::Lv2Host;

impl Lv2Host {
    /// `sys_fs_closedir` -- release a directory fd allocated via
    /// [`Self::dispatch_fs_opendir`].
    ///
    /// The file and directory fd stores are distinct, so a regular-file
    /// fd passed here surfaces CELL_EBADF.
    pub(in crate::host) fn dispatch_fs_closedir(&mut self, fd: u32) -> Lv2Dispatch {
        match self.fs_store_mut().close_dir(fd) {
            Ok(()) => Lv2Dispatch::immediate(0),
            Err(FsError::UnknownDir) => Lv2Dispatch::immediate(cell_errors::CELL_EBADF.into()),
            Err(other) => {
                self.record_invariant_break(
                    "dispatch.fs_closedir.unexpected_fs_error",
                    format_args!(
                        "FsStore::close_dir returned {other:?} for fd={fd:#x}; \
                         contract violated"
                    ),
                );
                Lv2Dispatch::immediate(cell_errors::CELL_EFAULT.into())
            }
        }
    }
}
