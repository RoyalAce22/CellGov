//! `sys_ppu_thread` priority arms.

use cellgov_effects::{Effect, WritePayload};
use cellgov_event::UnitId;
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
    /// priority to `*priop`.
    ///
    /// The id lookup precedes the `priop` null gate, so an unknown
    /// id answers ESRCH even when `priop` is null. Both codes are
    /// defined for this call; which one wins when both apply is a
    /// CellGov choice, unestablished against the console.
    ///
    /// # Errors
    ///
    /// - `CELL_EINVAL` when `priop` carries high bits, per
    ///   [`Lv2Host::narrow_u32_args`]. That gate precedes the id
    ///   lookup.
    /// - `CELL_ESRCH` when `thread_id` is absent from the thread table.
    /// - `CELL_EFAULT` when `priop` is null.
    pub(in crate::host::dispatch_route) fn dispatch_ppu_thread_get_priority(
        &mut self,
        args: [u64; 8],
        requester: UnitId,
        tick: GuestTicks,
    ) -> Lv2Dispatch {
        use cellgov_ps3_abi::lv2::syscall;

        let Some([priop]) =
            self.narrow_u32_args(syscall::PPU_THREAD_GET_PRIORITY, [("priop", args[1])])
        else {
            return Lv2Dispatch::immediate(errno::CELL_EINVAL.into());
        };
        let Some(thread) = self
            .state
            .ppu_threads
            .get(crate::ppu_thread::PpuThreadId::new(args[0]))
        else {
            return Lv2Dispatch::immediate(errno::CELL_ESRCH.into());
        };
        if let Some(d) = self.efault_if_null(&[priop]) {
            return d;
        }
        let write = Effect::shared_write(
            ByteRange::contiguous_u32(priop, 4),
            WritePayload::from_slice(&thread.attrs.priority.to_be_bytes()),
            requester,
            tick,
        );
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
    /// - `CELL_EINVAL` when `prio` is outside the window. `prio` is an
    ///   `int`, and the arm reads only the low word of its register.
    ///   The window test runs on that word, so the arm drops a high
    ///   word. Whether the kernel drops or refuses it is unestablished.
    /// - `CELL_ESRCH` when `thread_id` is absent from the thread table.
    pub(in crate::host::dispatch_route) fn dispatch_ppu_thread_set_priority(
        &mut self,
        args: [u64; 8],
    ) -> Lv2Dispatch {
        let prio = args[1] as i32;
        let floor = if self.debug_or_root() {
            PPU_THREAD_PRIORITY_MIN_ROOT
        } else {
            PPU_THREAD_PRIORITY_MIN
        };
        if !(floor..=PPU_THREAD_PRIORITY_MAX).contains(&prio) {
            return Lv2Dispatch::immediate(errno::CELL_EINVAL.into());
        }
        let id = crate::ppu_thread::PpuThreadId::new(args[0]);
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
