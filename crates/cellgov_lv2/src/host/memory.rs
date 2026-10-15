//! `sys_memory_allocate` and `sys_memory_free`, and the memory
//! containers `sys_memory_container_create` mints and
//! `sys_memory_container_destroy` removes.

use cellgov_event::UnitId;
use cellgov_mem::lanes::{LaneValue, ObjectLanes};
use cellgov_ps3_abi::lv2::errno;
use cellgov_ps3_abi::lv2::memory::{page_size, USER_MEMORY_TOTAL};

use crate::dispatch::Lv2Dispatch;
use crate::host::Lv2Host;
use cellgov_time::GuestTicks;

/// A memory container: its size and the part of it SPU thread groups and
/// allocations hold.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::host) struct MemoryContainer {
    /// The size the create asked for, rounded down to the 1 MiB granule.
    pub(in crate::host) size: u64,
    /// The memory the container's SPU thread groups and allocations
    /// hold.
    pub(in crate::host) taken: u64,
    /// Whether syscall 324 minted it. The `lv2_memory_free_container`
    /// capture shows `sys_memory_container_destroy` answering
    /// `CELL_ESRCH` for such a container: 324 mints its ids from a
    /// different range than 341, and the destroy does not find them.
    pub(in crate::host) from_324: bool,
}

impl MemoryContainer {
    /// An empty container of `size` bytes; `from_324` names the create
    /// that minted it.
    pub(in crate::host) fn with_size(size: u64, from_324: bool) -> Self {
        Self {
            size,
            taken: 0,
            from_324,
        }
    }

    /// Whether `bytes` more fit the container.
    pub(in crate::host) fn has_room(&self, bytes: u64) -> bool {
        self.size.saturating_sub(self.taken) >= bytes
    }
}

impl LaneValue for MemoryContainer {
    fn lanes(&self, lanes: &mut ObjectLanes) {
        lanes.lane(1, 0, self.size);
        lanes.lane(2, 0, self.taken);
        lanes.lane(3, 0, u64::from(self.from_324));
    }
}

/// A block `sys_memory_allocate` or `sys_memory_allocate_from_container`
/// handed out and no `sys_memory_free` returned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::host) struct MemoryBlock {
    pub(in crate::host) size: u32,
    /// The container the block came from; `None` for the process's own
    /// user memory.
    pub(in crate::host) container: Option<u32>,
}

impl LaneValue for MemoryBlock {
    fn lanes(&self, lanes: &mut ObjectLanes) {
        lanes.lane(1, 0, u64::from(self.size));
        lanes.lane(2, 0, self.container.map_or(0, |c| u64::from(c) + 1));
    }
}

impl Lv2Host {
    /// The user memory `sys_memory_get_user_memory_size` reports
    /// available: the total less every live block outside a container
    /// and every container's size. The `lv2_memory_free_container`
    /// capture shows both: a 1 MiB container create and a 1 MiB
    /// allocation each take 1 MiB from it, and a free or a destroy gives
    /// it back.
    pub(in crate::host) fn user_memory_available(&self) -> u32 {
        let blocks: u64 = self
            .state
            .memory_blocks
            .values()
            .filter(|b| b.container.is_none())
            .map(|b| u64::from(b.size))
            .sum();
        let containers: u64 = self.state.memory_containers.values().map(|c| c.size).sum();
        let held = u32::try_from(blocks.saturating_add(containers)).unwrap_or(u32::MAX);
        USER_MEMORY_TOTAL.saturating_sub(held)
    }

    /// Where a block of `size` bytes aligned to `align` goes: at the
    /// cursor while the user region has room past it, otherwise in the
    /// lowest hole a free left. A boot that never frees places every
    /// block where a plain bump allocator would.
    fn place_memory_block(&self, size: u32, align: u32) -> Option<u32> {
        let region_end = self
            .derived
            .mem_alloc_base
            .saturating_add(USER_MEMORY_TOTAL);
        let fits = |start: u32| start.checked_add(size).filter(|&end| end <= region_end);
        let align_up = |at: u32| at.checked_add(align - 1).map(|p| p & !(align - 1));
        if let Some(start) = align_up(self.state.mem_alloc_ptr) {
            if fits(start).is_some() {
                return Some(start);
            }
        }
        let mut cursor = self.derived.mem_alloc_base;
        for (addr, block) in self.state.memory_blocks.iter() {
            let start = align_up(cursor)?;
            if fits(start).is_some_and(|end| end <= addr) {
                return Some(start);
            }
            cursor = cursor.max(addr.saturating_add(block.size));
        }
        let start = align_up(cursor)?;
        fits(start).map(|_| start)
    }

    /// Records a block at `addr` and moves the cursor past it.
    fn commit_memory_block(&mut self, addr: u32, size: u32, container: Option<u32>) {
        self.state
            .memory_blocks
            .insert(addr, MemoryBlock { size, container });
        let end = addr.saturating_add(size);
        if end > self.state.mem_alloc_ptr {
            self.state.mem_alloc_ptr = end;
        }
    }

    /// `sys_memory_allocate` (348): places a block in the user region
    /// above the loaded image, aligned to the page size `flags` names,
    /// and writes its base to `*alloc_addr_ptr`.
    ///
    /// The page size is the granule of `flags`: 64 KiB for
    /// `SYS_MEMORY_PAGE_SIZE_64K`, 1 MiB for `SYS_MEMORY_PAGE_SIZE_1M`
    /// or a word naming none. The reference console returns a 1 MiB
    /// allocation on a 1 MiB boundary, and libgcm's init refuses an IO
    /// area that is not on one. The placement is CellGov's own: the base
    /// differs from the console's, and the console reuses a freed base
    /// at once where CellGov reuses one only when the region past the
    /// cursor is full.
    ///
    /// # Errors
    ///
    /// The list is in firing order. The `lv2_memory_allocate_order`
    /// capture shows that a size that is too large and no page multiple
    /// answers `CELL_ENOMEM`; the `lv2_memory_free_container` capture
    /// shows the zero size, the flags and the null pointer each refused
    /// alone. No capture combines bad flags with a size too large, so
    /// the flags gate leading is CellGov's choice.
    ///
    /// - `CELL_EINVAL` when the granularity field of `flags` holds an
    ///   encoding other than zero, 64 KiB or 1 MiB.
    /// - `CELL_ENOMEM` when `size` does not fit in `u32`, is more than
    ///   the available user memory, or no place in the user region
    ///   holds it.
    /// - `CELL_EALIGN` when `size` is zero or no multiple of the page
    ///   size, as the `lv2_error_codes_sync` capture shows.
    /// - `CELL_EFAULT` for a null `alloc_addr_ptr`, with nothing taken.
    pub(super) fn dispatch_memory_allocate(
        &mut self,
        size: u64,
        flags: u64,
        alloc_addr_ptr: u32,
        requester: UnitId,
        tick: GuestTicks,
    ) -> Lv2Dispatch {
        let Some(align) = page_size::accepted_granule(flags) else {
            return Lv2Dispatch::immediate(errno::CELL_EINVAL.into());
        };
        let Ok(size) = u32::try_from(size) else {
            return Lv2Dispatch::immediate(errno::CELL_ENOMEM.into());
        };
        if size > self.user_memory_available() {
            return Lv2Dispatch::immediate(errno::CELL_ENOMEM.into());
        }
        let Some(addr) = self.place_memory_block(size, align) else {
            return Lv2Dispatch::immediate(errno::CELL_ENOMEM.into());
        };
        if size == 0 || !size.is_multiple_of(align) {
            return Lv2Dispatch::immediate(errno::CELL_EALIGN.into());
        }
        if let Some(d) = self.efault_if_null(&[alloc_addr_ptr]) {
            return d;
        }
        self.commit_memory_block(addr, size, None);
        self.immediate_write_u32(addr, alloc_addr_ptr, requester, tick)
    }

    /// `sys_memory_allocate_from_container`: a `sys_memory_allocate`
    /// block taken from a container this process minted.
    ///
    /// The allocation takes `size` from the container, which an SPU
    /// thread group then cannot take, and nothing from the available
    /// user memory, which the container's size already left. A
    /// `sys_memory_free` of the block gives it back to the container.
    /// The console capture of `tests/micro/spu_group_container_shared`
    /// shows a container allocation leaving no room for a group.
    ///
    /// # Errors
    ///
    /// Listed in the order they fire, as the `lv2_memory_free_container`
    /// capture shows. `flags` names the page granule: libaudio.prx
    /// passes 0x200 here and rounds its request up to a 64 KiB multiple
    /// in the same call.
    ///
    /// - `CELL_ESRCH` for a container id no create minted.
    /// - `CELL_EINVAL` for `flags` other than 0, 64 KiB (0x200) or
    ///   1 MiB (0x400).
    /// - `CELL_EALIGN` when `size` is zero or not a multiple of the page
    ///   size.
    /// - `CELL_ENOMEM` when the container has no room for `size`, or no
    ///   place in the user region holds it.
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
        let Some(container) = self.state.memory_containers.get(cid).copied() else {
            return Lv2Dispatch::immediate(errno::CELL_ESRCH.into());
        };
        let align: u32 = match flags {
            0 | page_size::FLAG_1M => page_size::GRANULE_1M,
            page_size::FLAG_64K => page_size::GRANULE_64K,
            _ => return Lv2Dispatch::immediate(errno::CELL_EINVAL.into()),
        };
        if size == 0 || !size.is_multiple_of(u64::from(align)) {
            return Lv2Dispatch::immediate(errno::CELL_EALIGN.into());
        }
        if !container.has_room(size) {
            return Lv2Dispatch::immediate(errno::CELL_ENOMEM.into());
        }
        let Ok(size) = u32::try_from(size) else {
            return Lv2Dispatch::immediate(errno::CELL_ENOMEM.into());
        };
        let Some(addr) = self.place_memory_block(size, align) else {
            return Lv2Dispatch::immediate(errno::CELL_ENOMEM.into());
        };
        if let Some(d) = self.efault_if_null(&[alloc_addr_ptr]) {
            return d;
        }
        self.commit_memory_block(addr, size, Some(cid));
        if let Some(mut c) = self.state.memory_containers.get_mut(cid) {
            c.taken += u64::from(size);
        }
        self.immediate_write_u32(addr, alloc_addr_ptr, requester, tick)
    }

    /// `sys_memory_free` (349): returns the block at `addr` to the user
    /// memory or to its container.
    ///
    /// # Errors
    ///
    /// - `CELL_EINVAL` when `addr` is not the base of a live block: an
    ///   address no allocation returned, one already freed, or one
    ///   inside a block, as the `lv2_memory_free_container` capture
    ///   shows.
    pub(super) fn dispatch_memory_free(&mut self, addr: u32) -> Lv2Dispatch {
        let Some(block) = self.state.memory_blocks.remove(addr) else {
            return Lv2Dispatch::immediate(errno::CELL_EINVAL.into());
        };
        if let Some(cid) = block.container {
            if let Some(mut c) = self.state.memory_containers.get_mut(cid) {
                c.taken = c.taken.saturating_sub(u64::from(block.size));
            }
        }
        Lv2Dispatch::immediate(0)
    }

    /// `sys_memory_container_destroy` (342): removes a container that
    /// nothing holds memory from.
    ///
    /// # Errors
    ///
    /// Listed in the order they fire, as the `lv2_memory_free_container`
    /// capture shows.
    ///
    /// - `CELL_ESRCH` for an id no 341 create minted, one already
    ///   destroyed, or one syscall 324 minted.
    /// - `CELL_EBUSY` while an allocation or an SPU thread group holds
    ///   memory from the container. The capture shows the allocation;
    ///   the thread group is CellGov's reading of the same rule.
    pub(super) fn dispatch_memory_container_destroy(&mut self, cid: u32) -> Lv2Dispatch {
        match self.state.memory_containers.get(cid) {
            None => Lv2Dispatch::immediate(errno::CELL_ESRCH.into()),
            Some(c) if c.from_324 => Lv2Dispatch::immediate(errno::CELL_ESRCH.into()),
            Some(c) if c.taken != 0 => Lv2Dispatch::immediate(errno::CELL_EBUSY.into()),
            Some(_) => {
                self.state.memory_containers.remove(cid);
                Lv2Dispatch::immediate(0)
            }
        }
    }
}

#[cfg(test)]
#[path = "tests/memory_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/memory_container_tests.rs"]
mod container_tests;

#[cfg(test)]
#[path = "tests/memory_console_tests.rs"]
mod console_tests;

#[cfg(test)]
#[path = "tests/memory_free_tests.rs"]
mod free_tests;
