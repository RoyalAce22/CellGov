//! `sys_memory_allocate` bump-allocator dispatch.

use cellgov_event::UnitId;
use cellgov_ps3_abi::cell_errors;

use crate::dispatch::Lv2Dispatch;
use crate::host::Lv2Host;

/// Exclusive upper bound of the PS3 `main` region the LV2 allocator
/// may hand out from.
const MEM_ALLOC_REGION_END: u32 = 0x4000_0000;

impl Lv2Host {
    pub(super) fn dispatch_memory_allocate(
        &mut self,
        size: u64,
        alloc_addr_ptr: u32,
        requester: UnitId,
    ) -> Lv2Dispatch {
        // Every arithmetic step checked; the cursor is left unchanged
        // on ENOMEM.
        const ALIGN: u32 = 0x1_0000;
        let Ok(size) = u32::try_from(size) else {
            return Lv2Dispatch::immediate(cell_errors::CELL_ENOMEM.into());
        };
        let Some(aligned_ptr) = self
            .mem_alloc_ptr
            .checked_add(ALIGN - 1)
            .map(|p| p & !(ALIGN - 1))
        else {
            return Lv2Dispatch::immediate(cell_errors::CELL_ENOMEM.into());
        };
        let Some(next) = aligned_ptr.checked_add(size) else {
            return Lv2Dispatch::immediate(cell_errors::CELL_ENOMEM.into());
        };
        if next > MEM_ALLOC_REGION_END {
            return Lv2Dispatch::immediate(cell_errors::CELL_ENOMEM.into());
        }
        self.mem_alloc_ptr = next;
        self.immediate_write_u32(aligned_ptr, alloc_addr_ptr, requester)
    }
}

#[cfg(test)]
#[path = "tests/memory_tests.rs"]
mod tests;
