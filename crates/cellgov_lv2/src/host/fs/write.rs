//! `sys_fs_write` host dispatch.
//!
//! The `FsStore` model is read-only, so no fd in the store accepts a
//! write. The `lv2_fs_answers` capture reads what the console answers
//! a read-only fd: `CELL_EBADF` with a zero nwrite for a write of 4
//! bytes, and `CELL_OK` with a zero nwrite for a write of 0 bytes.
//! CellGov answers the same. A title that opens a file for writing
//! diverges earlier, at the open, which CellGov refuses with
//! `CELL_EROFS`. The one exception is a TTY-sink path
//! (`super::flags::FS_TTY_SINK_PATHS`): its open accepts write
//! access, but a write to that fd still answers `CELL_EBADF` here.

use cellgov_effects::{Effect, WritePayload};
use cellgov_event::UnitId;
use cellgov_mem::ByteRange;
use cellgov_ps3_abi::lv2::errno;

use super::ptr::out_ptr_writable;
use crate::dispatch::Lv2Dispatch;
use crate::host::{Lv2Host, Lv2Runtime};

impl Lv2Host {
    /// `sys_fs_write` -- every fd in the FS model is read-only.
    ///
    /// # Errors
    ///
    /// 1. `nwrite_ptr` misaligned / unwritable -> `CELL_EFAULT`, no effects.
    /// 2. `buf_ptr == 0` -> `CELL_EFAULT`, 8-byte zero write to `nwrite_ptr`.
    /// 3. fd not in FsStore -> `CELL_EBADF`, 8-byte zero write to `nwrite_ptr`.
    ///    The fd gate runs before the zero-size short-circuit, so a bad
    ///    fd is EBADF even when `size == 0`.
    /// 4. fd valid, `size == 0` -> `CELL_OK`, 8-byte zero write to `nwrite_ptr`.
    /// 5. fd valid, `size > 0` -> `CELL_EBADF`, 8-byte zero write to
    ///    `nwrite_ptr`: the answer to a fd opened without write access.
    ///    A fd opened with write flags on a TTY-sink path also answers
    ///    this, and the dropped write is logged as an invariant break.
    pub(in crate::host) fn dispatch_fs_write(
        &mut self,
        fd: u32,
        buf_ptr: u32,
        size: u64,
        nwrite_ptr: u32,
        requester: UnitId,
        rt: &dyn Lv2Runtime,
    ) -> Lv2Dispatch {
        if !out_ptr_writable(rt, nwrite_ptr, 8, 8) {
            return Lv2Dispatch::immediate(errno::CELL_EFAULT.into());
        }
        let nwrite_zero = Effect::shared_write(
            ByteRange::contiguous_u32(nwrite_ptr, 8),
            WritePayload::from_slice(&0u64.to_be_bytes()),
            requester,
            rt.current_tick(),
        );
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
        if self
            .fs_store()
            .fd_path(fd)
            .is_some_and(|path| super::flags::FS_TTY_SINK_PATHS.contains(&path))
        {
            self.log_invariant_break(
                "dispatch.fs_write.tty_sink_write_dropped",
                format_args!(
                    "sys_fs_write(fd={fd}, size={size:#x}): the fd was opened with write \
                     flags on a TTY-sink path, and no write path carries its bytes; \
                     returning CELL_EBADF"
                ),
            );
        }
        Lv2Dispatch::Immediate {
            code: errno::CELL_EBADF.into(),
            effects: vec![nwrite_zero],
        }
    }
}

#[cfg(test)]
#[path = "tests/write_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/tty_sink_write_tests.rs"]
mod tty_sink_tests;
