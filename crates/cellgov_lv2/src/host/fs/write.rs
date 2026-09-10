//! `sys_fs_write` host dispatch.
//!
//! The `FsStore` model is read-only: every `sys_fs_open` returns a
//! read-side fd, so no fd in the store could accept a write and this
//! is the null-backend arm of the FS surface.
//!
//! `CELL_EBADF` is the errno a non-writable fd earns, and CellGov
//! answers it for every write, so a guest cannot tell a read-only fd
//! from the absent write side. A title that opens a file for writing
//! diverges at its first write.

use cellgov_effects::{Effect, WritePayload};
use cellgov_event::{PriorityClass, UnitId};
use cellgov_mem::ByteRange;
use cellgov_ps3_abi::lv2::errno;

use crate::dispatch::Lv2Dispatch;
use crate::host::Lv2Host;
use cellgov_time::GuestTicks;

impl Lv2Host {
    /// `sys_fs_write` -- no writable fd in the FS model.
    ///
    /// # Errors
    ///
    /// 1. `nwrite_ptr == 0` -> `CELL_EFAULT`, no effects.
    /// 2. `buf_ptr == 0` -> `CELL_EFAULT`, 8-byte zero write to `nwrite_ptr`.
    /// 3. fd not in FsStore -> `CELL_EBADF`, 8-byte zero write to `nwrite_ptr`.
    ///    The fd gate runs before the zero-size short-circuit, so a bad
    ///    fd is EBADF even when `size == 0`.
    /// 4. fd valid, `size == 0` -> `CELL_OK`, 8-byte zero write to `nwrite_ptr`.
    /// 5. fd valid, `size > 0` -> `CELL_EBADF`, 8-byte zero write to `nwrite_ptr`,
    ///    plus `log_invariant_break`. This arm is the divergence the
    ///    module docs describe.
    pub(in crate::host) fn dispatch_fs_write(
        &mut self,
        fd: u32,
        buf_ptr: u32,
        size: u64,
        nwrite_ptr: u32,
        requester: UnitId,
        tick: GuestTicks,
    ) -> Lv2Dispatch {
        if nwrite_ptr == 0 {
            return Lv2Dispatch::immediate(errno::CELL_EFAULT.into());
        }
        let nwrite_zero = Effect::SharedWriteIntent {
            range: ByteRange::contiguous_u32(nwrite_ptr, 8),
            bytes: WritePayload::from_slice(&0u64.to_be_bytes()),
            ordering: PriorityClass::Normal,
            source: requester,
            source_time: tick,
        };
        if buf_ptr == 0 {
            return Lv2Dispatch::Immediate {
                code: errno::CELL_EFAULT.into(),
                effects: vec![nwrite_zero],
            };
        }
        // FsStore keeps file and dir fds in separate maps (`open_fds`
        // vs `open_dirs`) and `fstat` looks up only the first, so a dir
        // fd reads as UnknownFd here.
        if self.fs_store().fstat(fd).is_err() {
            return Lv2Dispatch::Immediate {
                code: errno::CELL_EBADF.into(),
                effects: vec![nwrite_zero],
            };
        }
        if size == 0 {
            return Lv2Dispatch::Immediate {
                code: 0,
                effects: vec![nwrite_zero],
            };
        }
        self.log_invariant_break(
            "dispatch.fs_write.read_only_model_rejects_write",
            format_args!(
                "sys_fs_write(fd={fd}, buf={buf_ptr:#010x}, size={size:#x}, \
                 nwrite={nwrite_ptr:#010x}): FsStore is read-only so no fd carries \
                 write access; returning CELL_EBADF, the code for an fd opened \
                 without write access"
            ),
        );
        Lv2Dispatch::Immediate {
            code: errno::CELL_EBADF.into(),
            effects: vec![nwrite_zero],
        }
    }
}

#[cfg(test)]
#[path = "tests/write_tests.rs"]
mod tests;
