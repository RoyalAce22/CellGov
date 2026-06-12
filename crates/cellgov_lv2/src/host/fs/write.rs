//! `sys_fs_write` host dispatch.
//!
//! The `FsStore` model is read-only: every `sys_fs_open` returns a
//! read-side fd. `sys_fs_write` is therefore the null-backend arm of
//! the FS surface -- there is no legitimate writer for any fd in the
//! store. Precedence and errno choices mirror RPCS3 `sys_fs.cpp`
//! `sys_fs_write` so the response is observably identical to the
//! oracle for the same guest input.

use cellgov_effects::{Effect, WritePayload};
use cellgov_event::{PriorityClass, UnitId};
use cellgov_mem::ByteRange;
use cellgov_ps3_abi::cell_errors;

use crate::dispatch::Lv2Dispatch;
use crate::host::Lv2Host;

impl Lv2Host {
    /// `sys_fs_write` -- no writable fd in the FS model.
    ///
    /// # Errors (precedence mirrors RPCS3 sys_fs.cpp:1206-1237)
    ///
    /// 1. `nwrite_ptr == 0` -> `CELL_EFAULT`, no effects.
    /// 2. `buf_ptr == 0` -> `CELL_EFAULT`, 8-byte zero write to `nwrite_ptr`.
    /// 3. fd not in FsStore -> `CELL_EBADF`, 8-byte zero write to `nwrite_ptr`.
    ///    Matches the `!file` half of `sys_fs.cpp:1219` which runs BEFORE the
    ///    `!nbytes` short-circuit at :1225 -- a bad fd is EBADF even when
    ///    `size == 0`.
    /// 4. fd valid, `size == 0` -> `CELL_OK`, 8-byte zero write to `nwrite_ptr`
    ///    (RPCS3 sys_fs.cpp:1225-1237; our model never sets `file->lock`,
    ///    so the EBUSY arm is dead).
    /// 5. fd valid, `size > 0` -> `CELL_EBADF`, 8-byte zero write to `nwrite_ptr`,
    ///    plus `log_invariant_break` documenting the read-only-model rejection.
    ///    Matches the `(nbytes && !(file->flags & CELL_FS_O_ACCMODE))` half of
    ///    `sys_fs.cpp:1219` for a read-only-opened file.
    pub(in crate::host) fn dispatch_fs_write(
        &mut self,
        fd: u32,
        buf_ptr: u32,
        size: u64,
        nwrite_ptr: u32,
        requester: UnitId,
    ) -> Lv2Dispatch {
        if nwrite_ptr == 0 {
            return Lv2Dispatch::immediate(cell_errors::CELL_EFAULT.into());
        }
        let nwrite_zero = Effect::SharedWriteIntent {
            range: ByteRange::contiguous_u32(nwrite_ptr, 8),
            bytes: WritePayload::from_slice(&0u64.to_be_bytes()),
            ordering: PriorityClass::Normal,
            source: requester,
            source_time: self.current_tick,
        };
        if buf_ptr == 0 {
            return Lv2Dispatch::Immediate {
                code: cell_errors::CELL_EFAULT.into(),
                effects: vec![nwrite_zero],
            };
        }
        // fd resolution precedes the `!nbytes` short-circuit per
        // sys_fs.cpp, so an unknown fd is EBADF regardless
        // of `size`. FsStore discriminates file vs dir fds at the
        // data-structure level (`open_fds` vs `open_dirs`); `fstat`
        // only looks up `open_fds`, so a dir fd reads as UnknownFd
        // here -- the same outcome RPCS3 produces via the
        // `idm::get_unlocked<lv2_fs_object, lv2_file>(fd)` downcast.
        if self.fs_store().fstat(fd).is_err() {
            return Lv2Dispatch::Immediate {
                code: cell_errors::CELL_EBADF.into(),
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
                 write access; returning CELL_EBADF (mirrors RPCS3 sys_fs.cpp \
                 for files without CELL_FS_O_ACCMODE access)"
            ),
        );
        Lv2Dispatch::Immediate {
            code: cell_errors::CELL_EBADF.into(),
            effects: vec![nwrite_zero],
        }
    }
}

#[cfg(test)]
#[path = "tests/write_tests.rs"]
mod tests;
