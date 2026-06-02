//! `sys_fs_read` host dispatch.

use cellgov_effects::{Effect, WritePayload};
use cellgov_event::{PriorityClass, UnitId};
use cellgov_mem::{ByteRange, GuestAddr};
use cellgov_ps3_abi::cell_errors;

use crate::dispatch::Lv2Dispatch;
use crate::host::{Lv2Host, Lv2Runtime};

use super::ptr::out_ptr_writable;

impl Lv2Host {
    /// `sys_fs_read` -- read up to `nbytes` from `fd`'s current
    /// offset into `buf_ptr`, advance the offset by the actual count
    /// returned, and write that count to `nread_out_ptr`.
    ///
    /// # Error precedence
    ///
    /// In order:
    /// 1. `nread_out_ptr` misaligned / unwritable -> CELL_EFAULT, no
    ///    effects.
    /// 2. Unknown `fd` -> CELL_EBADF, no effects (no out-pointer
    ///    write so the guest cannot mistake stale memory for nread).
    /// 3. `nbytes > 0` and `buf_ptr` unwritable for that span ->
    ///    CELL_EFAULT, no effects. Crucially, this happens BEFORE
    ///    the FS layer advances the offset; per POSIX, a failed
    ///    read must not change the file position.
    /// 4. Otherwise CELL_OK with up to two effects: the buffer
    ///    write (only if bytes were returned) and the nread write.
    pub(in crate::host) fn dispatch_fs_read(
        &mut self,
        fd: u32,
        buf_ptr: u32,
        nbytes: u64,
        nread_out_ptr: u32,
        requester: UnitId,
        rt: &dyn Lv2Runtime,
    ) -> Lv2Dispatch {
        // nread is a u64 (PS3 sys_fs_read signature: `u64 *nread`);
        // enforce 8-byte alignment and writability.
        if !out_ptr_writable(rt, nread_out_ptr, 8, 8) {
            return Lv2Dispatch::immediate(cell_errors::CELL_EFAULT.into());
        }

        // Peek fd validity without advancing the offset; fstat is
        // read-only and returns UnknownFd for an unknown fd.
        if self.fs_store().fstat(fd).is_err() {
            return Lv2Dispatch::immediate(cell_errors::CELL_EBADF.into());
        }

        let nbytes_usize = usize::try_from(nbytes).unwrap_or(usize::MAX);

        // Validate the destination buffer BEFORE the FS layer
        // advances the offset. POSIX requires a failed read leave
        // the file position unchanged; doing the writable check
        // after read_at would advance the offset and then return
        // EFAULT, which is a semantic break.
        if nbytes > 0 && !rt.writable(buf_ptr as u64, nbytes_usize) {
            return Lv2Dispatch::immediate(cell_errors::CELL_EFAULT.into());
        }

        // fstat said the fd is valid, so read_at must not surface
        // UnknownFd here. UnknownPath would mean the blob was
        // removed from under an open fd -- single-write
        // registration forbids that. Anything else is contract
        // drift in FsStore.
        let bytes_read = match self.fs_store_mut().read_at(fd, nbytes_usize) {
            Ok(b) => b,
            Err(other) => {
                self.record_invariant_break(
                    "dispatch.fs_read.unexpected_fs_error",
                    format_args!(
                        "FsStore::read_at returned {other:?} for fd={fd:#x} \
                         (fstat said valid); contract violated"
                    ),
                );
                return Lv2Dispatch::immediate(cell_errors::CELL_EFAULT.into());
            }
        };

        let nread = bytes_read.len() as u64;
        let tick = rt.current_tick();
        let mut effects = Vec::with_capacity(2);
        if !bytes_read.is_empty() {
            effects.push(Effect::SharedWriteIntent {
                range: ByteRange::new(GuestAddr::new(buf_ptr as u64), bytes_read.len() as u64)
                    .expect("buf_ptr range pre-validated by writable() above"),
                bytes: WritePayload::from_slice(&bytes_read),
                ordering: PriorityClass::Normal,
                source: requester,
                source_time: tick,
            });
        }
        effects.push(Effect::SharedWriteIntent {
            range: ByteRange::contiguous_u32(nread_out_ptr, 8),
            // PS3 is big-endian; guest reads via `ld`.
            bytes: WritePayload::from_slice(&nread.to_be_bytes()),
            ordering: PriorityClass::Normal,
            source: requester,
            source_time: tick,
        });
        Lv2Dispatch::Immediate { code: 0, effects }
    }
}
