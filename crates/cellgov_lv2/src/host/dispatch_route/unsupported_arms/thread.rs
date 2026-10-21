//! `sys_ppu_thread` priority arms.

use cellgov_effects::{Effect, WritePayload};
use cellgov_event::UnitId;
use cellgov_mem::ByteRange;
use cellgov_ps3_abi::lv2::errno;
use cellgov_ps3_abi::lv2::ppu_thread::{
    PPU_THREAD_PRIORITY_MAX, PPU_THREAD_PRIORITY_MAX_ROOT, PPU_THREAD_PRIORITY_MIN,
    PPU_THREAD_PRIORITY_MIN_ROOT,
};
use cellgov_time::GuestTicks;

use crate::dispatch::Lv2Dispatch;
use crate::host::Lv2Host;

impl Lv2Host {
    /// `sys_ppu_thread_get_priority` (48): writes the target's
    /// priority to `*priop`.
    ///
    /// The id lookup precedes the `priop` gate, so an unknown id
    /// answers ESRCH even when `priop` is null, as the
    /// `lv2_ppu_thread_answers` capture shows.
    ///
    /// # Errors
    ///
    /// - `CELL_ESRCH` when `thread_id` is absent from the thread table.
    /// - `CELL_EFAULT`, with no write, when `priop` is null or carries
    ///   high bits. The `lv2_arg_high_bits` capture reads CELL_EFAULT
    ///   for a valid pointer with bit 32 set.
    pub(in crate::host::dispatch_route) fn dispatch_ppu_thread_get_priority(
        &mut self,
        args: [u64; 8],
        requester: UnitId,
        tick: GuestTicks,
    ) -> Lv2Dispatch {
        let Some(thread) = self
            .state
            .ppu_threads
            .get(crate::ppu_thread::PpuThreadId::new(args[0]))
        else {
            return Lv2Dispatch::immediate(errno::CELL_ESRCH.into());
        };
        let priop = match u32::try_from(args[1]) {
            Ok(0) | Err(_) => return Lv2Dispatch::immediate(errno::CELL_EFAULT.into()),
            Ok(priop) => priop,
        };
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
    /// target's attrs; no scheduler consults it.
    ///
    /// A PPU thread priority runs 0 (highest) through 3071, and a
    /// value outside that window is EINVAL. A debug-or-root process
    /// has the wider window -512 through 3199 that the
    /// `lv2_ppu_thread_priority_bounds` and
    /// `lv2_ppu_thread_priority_ceiling` captures show.
    ///
    /// # Errors
    ///
    /// Listed in the order they fire, as the `lv2_ppu_thread_answers`
    /// capture shows.
    ///
    /// - `CELL_EINVAL` when `prio` is outside the window.
    /// - `CELL_ESRCH` when `thread_id` is absent from the thread table.
    ///
    /// `prio` is the low word of its register, per
    /// [`Lv2Host::low_i32_args`].
    pub(in crate::host::dispatch_route) fn dispatch_ppu_thread_set_priority(
        &mut self,
        args: [u64; 8],
        requester: UnitId,
    ) -> Lv2Dispatch {
        let [prio] = Self::low_i32_args([args[1]]);
        let window = if self.privilege_of(requester).debug_or_root() {
            PPU_THREAD_PRIORITY_MIN_ROOT..=PPU_THREAD_PRIORITY_MAX_ROOT
        } else {
            PPU_THREAD_PRIORITY_MIN..=PPU_THREAD_PRIORITY_MAX
        };
        if !window.contains(&prio) {
            return Lv2Dispatch::immediate(errno::CELL_EINVAL.into());
        }
        let id = crate::ppu_thread::PpuThreadId::new(args[0]);
        let Some(mut thread) = self.state.ppu_threads.get_mut(id) else {
            return Lv2Dispatch::immediate(errno::CELL_ESRCH.into());
        };
        thread.attrs.priority = prio as u32;
        Lv2Dispatch::immediate(0)
    }
}

#[cfg(test)]
#[path = "../../tests/ppu_thread_priority_tests.rs"]
mod ppu_thread_priority_tests;
