//! `sys_ppu_thread` priority arms.

use cellgov_effects::{Effect, WritePayload};
use cellgov_event::{PriorityClass, UnitId};
use cellgov_mem::ByteRange;
use cellgov_ps3_abi::lv2::errno;
use cellgov_ps3_abi::lv2::ppu_thread::{
    PPU_THREAD_PRIORITY_MAX, PPU_THREAD_PRIORITY_MIN, PPU_THREAD_PRIORITY_MIN_ROOT,
};
use cellgov_time::GuestTicks;

use crate::dispatch::Lv2Dispatch;
use crate::host::Lv2Host;

impl Lv2Host {
    /// `sys_ppu_thread_get_priority` (48): writes the target's
    /// priority to `*priop`, CELL_ESRCH for ids absent from the
    /// thread table.
    ///
    /// The id lookup precedes the `priop` null gate, so an unknown
    /// id answers ESRCH even when `priop` is null. Both codes are
    /// defined for this call; which one wins when both apply is a
    /// CellGov choice, unestablished against the console.
    pub(in crate::host::dispatch_route) fn dispatch_ppu_thread_get_priority(
        &self,
        args: [u64; 8],
        requester: UnitId,
        tick: GuestTicks,
    ) -> Lv2Dispatch {
        let thread_id = args[0] as u32;
        let priop = args[1] as u32;
        let Some(thread) = self
            .state
            .ppu_threads
            .get(crate::ppu_thread::PpuThreadId::new(thread_id as u64))
        else {
            return Lv2Dispatch::immediate(errno::CELL_ESRCH.into());
        };
        if let Some(d) = self.efault_if_null(&[priop]) {
            return d;
        }
        let write = Effect::SharedWriteIntent {
            range: ByteRange::contiguous_u32(priop, 4),
            bytes: WritePayload::from_slice(&thread.attrs.priority.to_be_bytes()),
            ordering: PriorityClass::Normal,
            source: requester,
            source_time: tick,
        };
        Lv2Dispatch::Immediate {
            code: 0,
            effects: vec![write],
        }
    }

    /// `sys_ppu_thread_set_priority` (47): stores `prio` in the
    /// target's attrs; the round-robin scheduler does not consult it.
    ///
    /// A PPU thread priority runs 0 (highest) through 3071, and a
    /// value outside that window is EINVAL. A debug-or-root process
    /// may go below zero, down to the -512 floor
    /// `_sys_ppu_thread_create` applies; that privileged widening
    /// has no public anchor.
    ///
    /// # Errors
    ///
    /// Listed in the order they fire.
    ///
    /// - `CELL_EINVAL` when `prio` is outside the window.
    /// - `CELL_ESRCH` when `thread_id` is absent from the thread table.
    pub(in crate::host::dispatch_route) fn dispatch_ppu_thread_set_priority(
        &mut self,
        args: [u64; 8],
    ) -> Lv2Dispatch {
        let thread_id = args[0] as u32;
        let prio = args[1] as i32;
        let floor = if self.debug_or_root() {
            PPU_THREAD_PRIORITY_MIN_ROOT
        } else {
            PPU_THREAD_PRIORITY_MIN
        };
        if !(floor..=PPU_THREAD_PRIORITY_MAX).contains(&prio) {
            return Lv2Dispatch::immediate(errno::CELL_EINVAL.into());
        }
        let id = crate::ppu_thread::PpuThreadId::new(thread_id as u64);
        let Some(thread) = self.state.ppu_threads.get_mut(id) else {
            return Lv2Dispatch::immediate(errno::CELL_ESRCH.into());
        };
        thread.attrs.priority = prio as u32;
        Lv2Dispatch::immediate(0)
    }
}

#[cfg(test)]
#[path = "../../tests/ppu_thread_priority_tests.rs"]
mod ppu_thread_priority_tests;
