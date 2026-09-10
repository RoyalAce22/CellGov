//! `sys_memory_allocate` bump-allocator dispatch.

use cellgov_event::UnitId;
use cellgov_ps3_abi::lv2::errno;

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
            return Lv2Dispatch::immediate(errno::CELL_ENOMEM.into());
        };
        let Some(aligned_ptr) = self
            .state
            .mem_alloc_ptr
            .checked_add(ALIGN - 1)
            .map(|p| p & !(ALIGN - 1))
        else {
            return Lv2Dispatch::immediate(errno::CELL_ENOMEM.into());
        };
        let Some(next) = aligned_ptr.checked_add(size) else {
            return Lv2Dispatch::immediate(errno::CELL_ENOMEM.into());
        };
        // The allocator's budget and sc 352's reported total are the
        // same number, so "this allocation succeeded" and "available
        // says there was room" can never contradict each other.
        let region_end = self
            .derived
            .mem_alloc_base
            .saturating_add(cellgov_ps3_abi::lv2::memory::USER_MEMORY_TOTAL);
        if next > region_end {
            return Lv2Dispatch::immediate(errno::CELL_ENOMEM.into());
        }
        self.state.mem_alloc_ptr = next;
        self.immediate_write_u32(aligned_ptr, alloc_addr_ptr, requester, tick)
    }

    /// `sys_memory_allocate_from_container`: the `sys_memory_allocate`
    /// bump allocator gated on a container id this process minted.
    ///
    /// Container budgets are not tracked, so a live container never
    /// runs out.
    ///
    /// # Errors
    ///
    /// Listed in the order they fire. `flags` names the page granule:
    /// libaudio.prx passes 0x200 here and rounds its request up to a
    /// 64 KiB multiple in the same call. The firing order itself is
    /// unestablished.
    ///
    /// - `CELL_EALIGN` for a zero `size`.
    /// - `CELL_EINVAL` for `flags` other than 0, 64 KiB (0x200), or
    ///   1 MiB (0x400).
    /// - `CELL_EALIGN` when `size` is not a multiple of the page size.
    /// - `CELL_ESRCH` for a container id no create minted.
    /// - `CELL_ENOMEM` when the user-memory budget is exhausted.
    /// - `CELL_EFAULT` for a null `alloc_addr_ptr`.
    pub(super) fn dispatch_memory_allocate_from_container(
        &mut self,
        size: u64,
        cid: u32,
        flags: u64,
        alloc_addr_ptr: u32,
        requester: UnitId,
        tick: GuestTicks,
    ) -> Lv2Dispatch {
        use cellgov_ps3_abi::lv2::memory::page_size;
        if size == 0 {
            return Lv2Dispatch::immediate(errno::CELL_EALIGN.into());
        }
        let align: u32 = match flags {
            0 | page_size::FLAG_1M => page_size::GRANULE_1M,
            page_size::FLAG_64K => page_size::GRANULE_64K,
            _ => return Lv2Dispatch::immediate(errno::CELL_EINVAL.into()),
        };
        if !size.is_multiple_of(u64::from(align)) {
            return Lv2Dispatch::immediate(errno::CELL_EALIGN.into());
        }
        if !self.state.memory_containers.contains(&cid) {
            return Lv2Dispatch::immediate(errno::CELL_ESRCH.into());
        }
        let Ok(size) = u32::try_from(size) else {
            return Lv2Dispatch::immediate(errno::CELL_ENOMEM.into());
        };
        let Some(aligned_ptr) = self
            .state
            .mem_alloc_ptr
            .checked_add(align - 1)
            .map(|p| p & !(align - 1))
        else {
            return Lv2Dispatch::immediate(errno::CELL_ENOMEM.into());
        };
        let Some(next) = aligned_ptr.checked_add(size) else {
            return Lv2Dispatch::immediate(errno::CELL_ENOMEM.into());
        };
        let region_end = self
            .derived
            .mem_alloc_base
            .saturating_add(cellgov_ps3_abi::lv2::memory::USER_MEMORY_TOTAL);
        if next > region_end {
            return Lv2Dispatch::immediate(errno::CELL_ENOMEM.into());
        }
        if let Some(d) = self.efault_if_null(&[alloc_addr_ptr]) {
            return d;
        }
        self.state.mem_alloc_ptr = next;
        self.immediate_write_u32(aligned_ptr, alloc_addr_ptr, requester, tick)
    }
}

#[cfg(test)]
#[path = "tests/memory_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/memory_container_tests.rs"]
mod container_tests;
