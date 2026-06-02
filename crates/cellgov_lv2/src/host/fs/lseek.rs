//! `sys_fs_lseek` host dispatch.

use cellgov_effects::{Effect, WritePayload};
use cellgov_event::{PriorityClass, UnitId};
use cellgov_mem::ByteRange;
use cellgov_ps3_abi::cell_errors;

use crate::dispatch::Lv2Dispatch;
use crate::fs_store::{FsError, SeekWhence};
use crate::host::{Lv2Host, Lv2Runtime};

use super::ptr::out_ptr_writable;

impl Lv2Host {
    /// `sys_fs_lseek` -- move `fd`'s offset to a new absolute
    /// position under SEEK_SET / SEEK_CUR / SEEK_END semantics and
    /// write that position to `pos_out_ptr`.
    ///
    /// # Errors
    ///
    /// In precedence order:
    /// 1. `pos_out_ptr` misaligned / unwritable -> CELL_EFAULT.
    /// 2. `whence` not in `{0, 1, 2}` -> CELL_EINVAL.
    /// 3. Unknown `fd` -> CELL_EBADF.
    /// 4. Seek lands outside `[0, u64::MAX]` -> CELL_EINVAL; the fd's
    ///    offset is unchanged.
    /// 5. Otherwise CELL_OK with one effect writing the new position
    ///    as a big-endian u64 at `pos_out_ptr`.
    pub(in crate::host) fn dispatch_fs_lseek(
        &mut self,
        fd: u32,
        offset: i64,
        whence: u32,
        pos_out_ptr: u32,
        requester: UnitId,
        rt: &dyn Lv2Runtime,
    ) -> Lv2Dispatch {
        if !out_ptr_writable(rt, pos_out_ptr, 8, 8) {
            return Lv2Dispatch::immediate(cell_errors::CELL_EFAULT.into());
        }

        let whence = match SeekWhence::from_guest(whence) {
            Some(w) => w,
            None => {
                return Lv2Dispatch::immediate(cell_errors::CELL_EINVAL.into());
            }
        };

        let new_pos = match self.fs_store_mut().seek(fd, offset, whence) {
            Ok(p) => p,
            Err(FsError::UnknownFd) => {
                return Lv2Dispatch::immediate(cell_errors::CELL_EBADF.into());
            }
            Err(FsError::SeekOutOfRange) => {
                return Lv2Dispatch::immediate(cell_errors::CELL_EINVAL.into());
            }
            Err(other) => {
                self.record_invariant_break(
                    "dispatch.fs_lseek.unexpected_fs_error",
                    format_args!(
                        "FsStore::seek returned {other:?} for fd={fd:#x} \
                         offset={offset} whence={whence:?}; contract violated"
                    ),
                );
                return Lv2Dispatch::immediate(cell_errors::CELL_EFAULT.into());
            }
        };

        let write = Effect::SharedWriteIntent {
            range: ByteRange::contiguous_u32(pos_out_ptr, 8),
            bytes: WritePayload::from_slice(&new_pos.to_be_bytes()),
            ordering: PriorityClass::Normal,
            source: requester,
            source_time: rt.current_tick(),
        };
        Lv2Dispatch::Immediate {
            code: 0,
            effects: vec![write],
        }
    }
}
