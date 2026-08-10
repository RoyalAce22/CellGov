//! `sys_memory_allocate` bump-allocator dispatch.

use cellgov_event::UnitId;
use cellgov_ps3_abi::cell_errors;

use crate::dispatch::Lv2Dispatch;
use crate::host::Lv2Host;
use cellgov_time::GuestTicks;

impl Lv2Host {
    pub(super) fn dispatch_memory_allocate(
        &mut self,
        size: u64,
        alloc_addr_ptr: u32,
        requester: UnitId,
        tick: GuestTicks,
    ) -> Lv2Dispatch {
        // The cursor is left unchanged on ENOMEM.
        const ALIGN: u32 = 0x1_0000;
        let Ok(size) = u32::try_from(size) else {
            return Lv2Dispatch::immediate(cell_errors::CELL_ENOMEM.into());
        };
        let Some(aligned_ptr) = self
            .state
            .mem_alloc_ptr
            .checked_add(ALIGN - 1)
            .map(|p| p & !(ALIGN - 1))
        else {
            return Lv2Dispatch::immediate(cell_errors::CELL_ENOMEM.into());
        };
        let Some(next) = aligned_ptr.checked_add(size) else {
            return Lv2Dispatch::immediate(cell_errors::CELL_ENOMEM.into());
        };
        // The allocator's budget and sc 352's reported total are the
        // same number, so "this allocation succeeded" and "available
        // says there was room" can never contradict each other.
        let region_end = self
            .derived
            .mem_alloc_base
            .saturating_add(cellgov_ps3_abi::sys_memory::USER_MEMORY_TOTAL);
        if next > region_end {
            return Lv2Dispatch::immediate(cell_errors::CELL_ENOMEM.into());
        }
        self.state.mem_alloc_ptr = next;
        self.immediate_write_u32(aligned_ptr, alloc_addr_ptr, requester, tick)
    }
}

#[cfg(test)]
#[path = "tests/memory_tests.rs"]
mod tests;
