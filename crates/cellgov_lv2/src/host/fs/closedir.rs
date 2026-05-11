//! `sys_fs_closedir` host dispatch.

use cellgov_ps3_abi::cell_errors as errno;

use crate::dispatch::Lv2Dispatch;
use crate::fs_store::FsError;
use crate::host::Lv2Host;

impl Lv2Host {
    /// `sys_fs_closedir` -- release a directory fd allocated via
    /// [`Self::dispatch_fs_opendir`]. Returns CELL_EBADF for an
    /// unknown directory fd OR for a value that names a regular
    /// file fd; the two stores are deliberately distinct so a
    /// guest that mixes them sees the error rather than silently
    /// closing the wrong handle.
    pub(in crate::host) fn dispatch_fs_closedir(&mut self, fd: u32) -> Lv2Dispatch {
        match self.fs_store_mut().close_dir(fd) {
            Ok(()) => Lv2Dispatch::Immediate {
                code: 0,
                effects: vec![],
            },
            Err(FsError::UnknownDir) => Lv2Dispatch::Immediate {
                code: errno::CELL_EBADF.into(),
                effects: vec![],
            },
            Err(other) => {
                self.record_invariant_break(
                    "dispatch.fs_closedir.unexpected_fs_error",
                    format_args!(
                        "FsStore::close_dir returned {other:?} for fd={fd:#x}; \
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
