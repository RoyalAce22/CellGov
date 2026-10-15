//! LV2 dispatch for a PPU's access to an SPU thread's local store:
//! `sys_spu_thread_write_ls` and `sys_spu_thread_read_ls`.
//!
//! Both check, in order: the size is 1, 2, 4 or 8 bytes and the
//! address is inside local store and aligned to it (`CELL_EINVAL`),
//! then that an initialized thread has the id (`CELL_ESRCH`), then that
//! its group runs (`CELL_ESTAT`); a suspended group runs, and so does a
//! group one of whose threads exited. The kernel reads the low 32
//! bits of the address. The spu_thread_ls console capture fixes the
//! order and each answer.

use cellgov_effects::{Effect, WritePayload};
use cellgov_event::UnitId;
use cellgov_mem::ByteRange;
use cellgov_ps3_abi::hw::spu::SPU_LS_SIZE;
use cellgov_ps3_abi::lv2::errno::{self, Lv2ErrCode};

use crate::dispatch::Lv2Dispatch;
use crate::host::{Lv2Host, Lv2Runtime};
use crate::thread_group::{GroupState, MAX_SLOTS_PER_GROUP};

/// The size in bytes of an access of `size` at `lsa`, or `None` when
/// the size is not 1, 2, 4 or 8, or the address is outside local store
/// or not aligned to the size.
fn access_len(lsa: u32, size: u64) -> Option<u32> {
    let len @ (1 | 2 | 4 | 8) = size else {
        return None;
    };
    let len = len as u32;
    let in_range = (lsa as usize) < SPU_LS_SIZE && lsa.is_multiple_of(len);
    in_range.then_some(len)
}

impl Lv2Host {
    /// The unit of `thread_id`, or the errno the access answers:
    /// `CELL_ESRCH` when no initialized thread has the id, `CELL_ESTAT`
    /// when its group does not run. A thread that exited while its
    /// group runs keeps its local store.
    fn local_store_target(&self, thread_id: u32) -> Result<UnitId, Lv2ErrCode> {
        let groups = &self.state.groups;
        if groups.signal_config(thread_id).is_none() {
            return Err(errno::CELL_ESRCH);
        }
        let running = groups
            .get(thread_id / MAX_SLOTS_PER_GROUP)
            .is_some_and(|group| group.state == GroupState::Running);
        if !running {
            return Err(errno::CELL_ESTAT);
        }
        groups.unit_for_thread(thread_id).ok_or(errno::CELL_ESTAT)
    }

    /// `sys_spu_thread_write_ls`
    /// ([`cellgov_ps3_abi::lv2::syscall::SPU_THREAD_WRITE_LS`]): write
    /// the low `size` bytes of `value`, big-endian, at `lsa` in the local
    /// store of the SPU that runs `thread_id`.
    ///
    /// # Errors
    ///
    /// `CELL_EINVAL`, `CELL_ESRCH` and `CELL_ESTAT` in the module's
    /// order.
    pub(super) fn dispatch_write_ls(
        &self,
        thread_id: u32,
        lsa: u32,
        value: u64,
        size: u64,
        requester: UnitId,
    ) -> Lv2Dispatch {
        let Some(len) = access_len(lsa, size) else {
            return Lv2Dispatch::immediate(errno::CELL_EINVAL.into());
        };
        let target = match self.local_store_target(thread_id) {
            Ok(target) => target,
            Err(code) => return Lv2Dispatch::immediate(code.into()),
        };
        let bytes = value.to_be_bytes()[8 - len as usize..].to_vec();
        Lv2Dispatch::Immediate {
            code: 0,
            effects: vec![Effect::SpuLocalStoreWrite {
                target,
                lsa,
                bytes,
                source: requester,
            }],
        }
    }

    /// `sys_spu_thread_read_ls`
    /// ([`cellgov_ps3_abi::lv2::syscall::SPU_THREAD_READ_LS`]): read
    /// `size` bytes at `lsa` from the local store of the SPU that runs
    /// `thread_id`, and write them zero-extended, as a big-endian 64-bit
    /// word, to `value_ptr`.
    ///
    /// # Errors
    ///
    /// `CELL_EINVAL`, `CELL_ESRCH` and `CELL_ESTAT` in the module's
    /// order, then `CELL_EFAULT` when `value_ptr` is null or the 8 bytes
    /// at it are not writable.
    pub(super) fn dispatch_read_ls(
        &mut self,
        thread_id: u32,
        lsa: u32,
        value_ptr: u32,
        size: u64,
        requester: UnitId,
        rt: &dyn Lv2Runtime,
    ) -> Lv2Dispatch {
        let Some(len) = access_len(lsa, size) else {
            return Lv2Dispatch::immediate(errno::CELL_EINVAL.into());
        };
        let target = match self.local_store_target(thread_id) {
            Ok(target) => target,
            Err(code) => return Lv2Dispatch::immediate(code.into()),
        };
        if value_ptr == 0 || !rt.writable(u64::from(value_ptr), 8) {
            return Lv2Dispatch::immediate(errno::CELL_EFAULT.into());
        }
        let Some(bytes) = rt.read_local_store(target, lsa, len) else {
            self.log_invariant_break(
                "dispatch.read_ls_target_without_local_store",
                format_args!(
                    "sys_spu_thread_read_ls: thread {thread_id:#x} runs on {target:?}, \
                     which has no local store; returning CELL_ESTAT"
                ),
            );
            return Lv2Dispatch::immediate(errno::CELL_ESTAT.into());
        };
        let mut word = [0u8; 8];
        word[8 - bytes.len()..].copy_from_slice(&bytes);
        Lv2Dispatch::Immediate {
            code: 0,
            effects: vec![
                Effect::SpuLocalStoreRead {
                    target,
                    lsa,
                    len,
                    source: requester,
                },
                Effect::shared_write(
                    ByteRange::contiguous_u32(value_ptr, 8),
                    WritePayload::from_slice(&word),
                    requester,
                    rt.current_tick(),
                ),
            ],
        }
    }
}

#[cfg(test)]
#[path = "tests/spu_local_store_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/spu_thread_ls_console_tests.rs"]
mod console_tests;
