//! Memory allocation: the allocation base, the RSX seeds, the id allocator and the mmapper handout, search and ledger.

use std::collections::BTreeMap;

use crate::host::rsx::SysRsxContext;

use super::model::Lv2Host;

impl Lv2Host {
    /// Callers that load a real ELF must set this to the
    /// 64KB-aligned address above the ELF's highest PT_LOAD end;
    /// the default (`0x0001_0000`) overwrites the image otherwise.
    pub fn set_mem_alloc_base(&mut self, base: u32) {
        debug_assert!(
            base & 0xFFFF == 0,
            "mem_alloc_base must be 64 KiB aligned, got {base:#x}",
        );
        debug_assert!(
            base >= 0x0001_0000,
            "mem_alloc_base must sit at or above the PS3 user-memory floor (0x0001_0000), got {base:#x}",
        );
        debug_assert!(
            base < Self::SYS_RSX_MEM_BASE,
            "mem_alloc_base must sit below SYS_RSX_MEM_BASE ({:#x}), got {base:#x}",
            Self::SYS_RSX_MEM_BASE,
        );
        self.state.mem_alloc_ptr = base;
        self.derived.mem_alloc_base = base;
    }

    /// sys_rsx host context.
    #[inline]
    pub fn sys_rsx_context(&self) -> &SysRsxContext {
        &self.state.rsx_context
    }

    /// Record an iomap mapping without going through 672.
    ///
    /// Unvalidated seeding hook for synthetic scenarios;
    /// `dispatch_sys_rsx_context_iomap` is the path that checks the
    /// 672 contract.
    pub fn seed_rsx_iomap(&mut self, io: u32, ea: u32, size: u32) {
        self.state.rsx_context.iomap_io = io;
        self.state.rsx_context.iomap_ea = ea;
        self.state.rsx_context.iomap_size = size;
    }

    /// Mark the sys_rsx context as allocated under `context_id`
    /// without going through 670.
    ///
    /// Satisfies the `allocated && matching id` guard at the top of
    /// `sys_rsx_context_attribute` (674) without the OUT-pointer
    /// memory plumbing 670 requires;
    /// `dispatch_sys_rsx_context_allocate` is the validated path.
    pub fn seed_rsx_context_allocated(&mut self, context_id: u32) {
        self.state.rsx_context.allocated = true;
        self.state.rsx_context.context_id = context_id;
    }

    pub(in crate::host) fn alloc_id(&mut self) -> u32 {
        let id = self.state.next_kernel_id;
        self.state.next_kernel_id = self
            .state
            .next_kernel_id
            .checked_add(1)
            .expect("kernel id space exhausted");
        id
    }

    /// Bump the mmapper VM cursor by `size` rounded up to the
    /// 256 MiB granule and return the pre-bump cursor.
    ///
    /// Returns `None` for `size == 0`, when the bump would overflow
    /// `u32`, or when the resulting range would cross
    /// [`Self::MMAPPER_REGION_END`].
    pub(in crate::host) fn mmapper_alloc(&mut self, size: u32) -> Option<u32> {
        if size == 0 {
            return None;
        }
        let granule = u32::try_from(cellgov_ps3_abi::lv2::memory::VM_AREA_GRANULE).ok()?;
        let rounded = size.checked_add(granule - 1)? & !(granule - 1);
        let base = self.state.mmapper_addr_cursor;
        let next = base.checked_add(rounded)?;
        if next > Self::MMAPPER_REGION_END {
            return None;
        }
        self.state.mmapper_addr_cursor = next;
        Some(base)
    }

    /// Search for the first free, `align`-aligned range of `size`
    /// bytes inside `[MMAPPER_REGION_START, MMAPPER_REGION_END)` at
    /// or after `hint`, skipping every range recorded in
    /// [`mmapper_install_ledger`](crate::host::derived::Lv2Derived::mmapper_install_ledger) and every window occupied in
    /// the caller's committed layout. Loader regions are invisible to
    /// the ledger, so an occupied window is skipped rather than
    /// refused -- `sys_mmapper_search_and_map` searches, it does not
    /// place.
    ///
    /// `hint` is rounded UP to `align`; misaligned hints do not fail.
    /// The scan begins at that rounded hint, clamped up to
    /// `MMAPPER_REGION_START`. A caller's `start_addr` therefore
    /// steers where the search starts; it does not only name a
    /// region. The kernel's own placement rule is unestablished.
    ///
    /// Returns `None` on exhaustion, which the caller answers with
    /// `CELL_ENOMEM`.
    pub(in crate::host) fn mmapper_search_free_range(
        &self,
        hint: u32,
        size: u32,
        align: u32,
        rt: &dyn crate::host::Lv2Runtime,
    ) -> Option<u32> {
        debug_assert!(
            align.is_power_of_two(),
            "mmapper align must be a power of two"
        );
        debug_assert!(align != 0, "mmapper align must be non-zero");
        if size == 0 {
            return None;
        }
        let align_mask = align - 1;
        let hint_clamped = hint.max(Self::MMAPPER_REGION_START);
        let mut candidate = hint_clamped.checked_add(align_mask)? & !align_mask;
        loop {
            let end = candidate.checked_add(size)?;
            if end > Self::MMAPPER_REGION_END {
                return None;
            }
            // Every ledger entry starting below `end` is a possible
            // overlap, not only the nearest one: a map whose runtime
            // `install_region` was refused still left its ledger
            // entry behind, so a later map can record a range nested
            // inside a longer one. O(ledger) per step, and the ledger
            // holds one entry per shm map the boot made.
            let ledger_skip_to = self
                .derived
                .mmapper_install_ledger
                .range(..end)
                // An entry whose end overflows u32 covers the rest of
                // the address space; saturating keeps it an obstacle
                // instead of letting the overflow read as free.
                .map(|(&start, &len)| start.saturating_add(len))
                .filter(|&prior_end| prior_end > candidate)
                .max();
            let committed_skip_to = rt
                .committed_overlap_end(u64::from(candidate), u64::from(size))
                .map(|e| u32::try_from(e).unwrap_or(u32::MAX));
            debug_assert!(
                committed_skip_to.is_none_or(|e| e > candidate),
                "committed_overlap_end({candidate:#x}, {size:#x}) answered \
                 {committed_skip_to:?}, which does not lie past the window it \
                 claims to overlap",
            );
            // Both arms are filtered to ends strictly above
            // `candidate`, so the re-aligned candidate strictly
            // increases every step and the loop terminates.
            match ledger_skip_to
                .into_iter()
                .chain(committed_skip_to.filter(|&e| e > candidate))
                .max()
            {
                Some(skip_to) => {
                    candidate = skip_to.checked_add(align_mask)? & !align_mask;
                }
                None => return Some(candidate),
            }
        }
    }

    /// Record an mmapper-window install in the host ledger. Paired
    /// with a `PendingRegionInstall` push by the same dispatch.
    pub(in crate::host) fn mmapper_ledger_insert(&mut self, addr: u32, size: u32) {
        let prior = self.derived.mmapper_install_ledger.insert(addr, size);
        debug_assert!(
            prior.is_none(),
            "mmapper ledger: addr {addr:#x} already recorded (size {prior:?})",
        );
    }

    /// `ipc_key -> mem_id` registrations made by keyed 332 calls.
    pub fn mmapper_ipc(&self) -> &BTreeMap<u64, u32> {
        &self.state.mmapper_ipc
    }
}
