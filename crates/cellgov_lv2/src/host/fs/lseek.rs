//! `sys_fs_lseek` host dispatch.

use cellgov_effects::{Effect, WritePayload};
use cellgov_event::{PriorityClass, UnitId};
use cellgov_mem::{ByteRange, GuestAddr};
use cellgov_ps3_abi::cell_errors as errno;

use crate::dispatch::Lv2Dispatch;
use crate::fs_store::{FsError, SeekWhence};
use crate::host::{Lv2Host, Lv2Runtime};

use super::ptr::out_ptr_writable;

impl Lv2Host {
    /// `sys_fs_lseek` -- move `fd`'s offset to a new absolute
    /// position under SEEK_SET / SEEK_CUR / SEEK_END semantics and
    /// write that position to `pos_out_ptr`.
    ///
    /// # Error precedence
    ///
    /// In order:
    /// 1. `pos_out_ptr` misaligned / unwritable -> CELL_EFAULT, no
    ///    effects. We bail before touching the fd table because no
    ///    other error can be reported (the new position would have
    ///    nowhere to land).
    /// 2. `whence` not in `{0, 1, 2}` -> CELL_EINVAL, no effects.
    ///    Cheap argument check before fd lookup.
    /// 3. Unknown `fd` -> CELL_EBADF, no effects.
    /// 4. Seek lands outside `[0, u64::MAX]` (negative-past-zero or
    ///    positive overflow) -> CELL_EINVAL, no effects. The fd's
    ///    offset is unchanged on this path; FsStore::seek validates
    ///    before mutating.
    /// 5. Otherwise CELL_OK with one effect: the new position
    ///    written as a big-endian u64 at `pos_out_ptr`.
    pub(in crate::host) fn dispatch_fs_lseek(
        &mut self,
        fd: u32,
        offset: i64,
        whence: u32,
        pos_out_ptr: u32,
        requester: UnitId,
        rt: &dyn Lv2Runtime,
    ) -> Lv2Dispatch {
        // pos is a u64 (PSL1GHT signature: `u64 *pos`); enforce
        // 8-byte alignment and writability before any fd touch.
        if !out_ptr_writable(rt, pos_out_ptr, 8, 8) {
            return Lv2Dispatch::Immediate {
                code: errno::CELL_EFAULT.into(),
                effects: vec![],
            };
        }

        // Decode whence; out-of-range is CELL_EINVAL with no
        // out-pointer write. Done before fd lookup so a probe with
        // garbage whence does not need a valid fd to surface
        // EINVAL.
        let whence = match SeekWhence::from_guest(whence) {
            Some(w) => w,
            None => {
                return Lv2Dispatch::Immediate {
                    code: errno::CELL_EINVAL.into(),
                    effects: vec![],
                };
            }
        };

        let new_pos = match self.fs_store_mut().seek(fd, offset, whence) {
            Ok(p) => p,
            Err(FsError::UnknownFd) => {
                return Lv2Dispatch::Immediate {
                    code: errno::CELL_EBADF.into(),
                    effects: vec![],
                };
            }
            Err(FsError::SeekOutOfRange) => {
                return Lv2Dispatch::Immediate {
                    code: errno::CELL_EINVAL.into(),
                    effects: vec![],
                };
            }
            Err(other) => {
                // seek's contract: only UnknownFd / SeekOutOfRange
                // / UnknownPath. UnknownPath would mean the blob
                // disappeared from under an open fd -- single-write
                // registration forbids that. Anything else is
                // FsError surface drift.
                self.record_invariant_break(
                    "dispatch.fs_lseek.unexpected_fs_error",
                    format_args!(
                        "FsStore::seek returned {other:?} for fd={fd:#x} \
                         offset={offset} whence={whence:?}; contract violated"
                    ),
                );
                return Lv2Dispatch::Immediate {
                    code: errno::CELL_EFAULT.into(),
                    effects: vec![],
                };
            }
        };

        let write = Effect::SharedWriteIntent {
            range: ByteRange::new(GuestAddr::new(pos_out_ptr as u64), 8)
                .expect("pos_out_ptr range pre-validated by writable() above"),
            // PS3 is big-endian; guest reads via `ld`.
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
