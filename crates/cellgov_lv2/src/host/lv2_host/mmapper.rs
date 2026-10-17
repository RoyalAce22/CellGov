//! Memory allocation: the allocation base, the RSX seeds, the id allocator and the mmapper handout, search and ledger.

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

    /// Mint a kernel id of `class` from the shared slot table; `None`
    /// once the process holds the most ids it may, which the create
    /// answers with `CELL_EAGAIN`.
    pub(in crate::host) fn alloc_id(&mut self, class: u32) -> Option<u32> {
        self.state.kernel_ids.alloc(class)
    }

    /// Return `id` to the slot table once a destroy removes its object.
    ///
    /// Every id a destroy path frees came from [`Self::alloc_id`], so
    /// an unknown id is an invariant break, not a guest error.
    pub(in crate::host) fn release_id(&mut self, id: u32) {
        if !self.state.kernel_ids.release(id) {
            self.log_invariant_break(
                "kernel_ids.release_unknown",
                format_args!("released kernel id {id:#x} that the slot table does not hold"),
            );
        }
    }

    /// Mint a memory container id: 324's or 341's base plus the next
    /// serial, skipping a serial a live container still holds.
    pub(in crate::host) fn alloc_container_id(&mut self, from_324: bool) -> u32 {
        use cellgov_ps3_abi::lv2::kernel_id::{
            MEMORY_CONTAINER_324_BASE, MEMORY_CONTAINER_BASE, MEMORY_CONTAINER_SERIAL_MASK,
        };
        let base = if from_324 {
            MEMORY_CONTAINER_324_BASE
        } else {
            MEMORY_CONTAINER_BASE
        };
        loop {
            let serial = self.state.next_container_serial & MEMORY_CONTAINER_SERIAL_MASK;
            self.state.next_container_serial = (serial + 1) & MEMORY_CONTAINER_SERIAL_MASK;
            let held = |b: u32| self.state.memory_containers.contains_key(b | serial);
            if !held(MEMORY_CONTAINER_BASE) && !held(MEMORY_CONTAINER_324_BASE) {
                return base | serial;
            }
        }
    }

    /// Reserve the lowest free range of `size`, rounded up to the
    /// 256 MiB granule, in `[MMAPPER_REGION_START, MMAPPER_REGION_END)`,
    /// record it as an area of `page`-byte pages, and return its base.
    ///
    /// A base that 331 freed is free for the next 330: the
    /// `lv2_mmapper_map_edges` capture shows a 330 after a 331 getting
    /// the freed base back.
    ///
    /// Returns `None` for `size == 0`, when the rounding would overflow
    /// `u32`, or when no free range below
    /// [`Self::MMAPPER_REGION_END`] holds the request.
    pub(in crate::host) fn mmapper_alloc(&mut self, size: u32, page: u32) -> Option<u32> {
        if size == 0 {
            return None;
        }
        let granule = u32::try_from(cellgov_ps3_abi::lv2::memory::VM_AREA_GRANULE).ok()?;
        let rounded = size.checked_add(granule - 1)? & !(granule - 1);
        let mut base = u64::from(Self::MMAPPER_REGION_START);
        for (area_base, area) in self.state.mmapper_areas.iter() {
            if u64::from(area_base) >= base + u64::from(rounded) {
                break;
            }
            base = base.max(u64::from(area_base) + u64::from(area.size));
        }
        if base + u64::from(rounded) > u64::from(Self::MMAPPER_REGION_END) {
            return None;
        }
        let base = u32::try_from(base).ok()?;
        self.state.mmapper_areas.insert(
            base,
            crate::host::mmapper::MmapperArea {
                size: rounded,
                page,
            },
        );
        Some(base)
    }

    /// Search for the first free, `align`-aligned range of `size`
    /// bytes inside `[MMAPPER_REGION_START, MMAPPER_REGION_END)` at
    /// or after `hint` and ending at or below `limit`, skipping every
    /// window in [`mmapper_windows`](crate::host::state::Lv2State::mmapper_windows)
    /// and every range occupied in the caller's committed layout.
    /// Loader regions are invisible to the window table, so an occupied
    /// range is skipped rather than refused -- `sys_mmapper_search_and_map` searches, it does not
    /// place.
    ///
    /// `hint` is rounded UP to `align`; misaligned hints do not fail.
    /// The scan begins at that rounded hint, clamped up to
    /// `MMAPPER_REGION_START`. A caller's `start_addr` therefore
    /// is the base of the area the search stays inside. The
    /// `lv2_mmapper_map_64k` capture shows two searches from one base
    /// landing at the base and one segment above it.
    ///
    /// Returns `None` on exhaustion, which the caller answers with
    /// `CELL_ENOMEM`.
    pub(in crate::host) fn mmapper_search_free_range(
        &self,
        hint: u32,
        size: u32,
        align: u32,
        limit: u32,
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
            if end > Self::MMAPPER_REGION_END || end > limit {
                return None;
            }
            let ledger_skip_to = self
                .mmapper_window_ends_below(end)
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

    /// The area 330 reserved that holds `addr`: its base and record.
    pub(in crate::host) fn mmapper_area_of(
        &self,
        addr: u64,
    ) -> Option<(u32, crate::host::mmapper::MmapperArea)> {
        let addr = u32::try_from(addr).ok()?;
        self.state
            .mmapper_areas
            .iter()
            .rev()
            .find(|&(base, _)| base <= addr)
            .filter(|&(base, area)| u64::from(addr) < u64::from(base) + u64::from(area.size))
            .map(|(base, &area)| (base, area))
    }

    /// The end of every window that starts below `end`.
    ///
    /// Every such window is a possible overlap, not only the nearest
    /// one: a map whose runtime `install_region` was refused still
    /// left its window behind, so a later map can record a range nested
    /// inside a longer one. O(windows), one per live shm map. A window
    /// whose end overflows u32 covers the rest of the address space;
    /// saturating keeps it an obstacle instead of letting the overflow
    /// read as free.
    pub(in crate::host) fn mmapper_window_ends_below(
        &self,
        end: u32,
    ) -> impl Iterator<Item = u32> + '_ {
        self.state
            .mmapper_windows
            .iter()
            .take_while(move |&(start, _)| start < end)
            .map(|(start, w)| start.saturating_add(w.size))
    }

    /// Record a window 334 or 337 mapped. Paired with a
    /// `PendingRegionInstall` push by the same dispatch.
    pub(in crate::host) fn mmapper_window_insert(&mut self, addr: u32, size: u32, mem_id: u32) {
        let prior = self
            .state
            .mmapper_windows
            .insert(addr, crate::host::mmapper::MmapperWindow { mem_id, size });
        debug_assert!(
            prior.is_none(),
            "mmapper window: addr {addr:#x} already recorded ({prior:?})",
        );
    }

    /// Whether a window maps `mem_id`, or a refused 337 left a map of it
    /// behind: what holds the segment against a free.
    pub(in crate::host) fn mmapper_segment_busy(
        &self,
        mem_id: u32,
        handle: crate::host::mmapper::MmapperHandle,
    ) -> bool {
        handle.stuck_maps != 0
            || self
                .state
                .mmapper_windows
                .values()
                .any(|w| w.mem_id == mem_id)
    }

    /// Whether a window lies in the area at `base`: what holds the area
    /// against a free.
    pub(in crate::host) fn mmapper_area_busy(
        &self,
        base: u32,
        area: crate::host::mmapper::MmapperArea,
    ) -> bool {
        let end = u64::from(base) + u64::from(area.size);
        self.state
            .mmapper_windows
            .keys()
            .any(|start| start >= base && u64::from(start) < end)
    }

    /// `ipc_key -> mem_id` registrations made by keyed 332 calls.
    pub fn mmapper_ipc(&self) -> &cellgov_mem::lanes::LaneMap<u64, u32> {
        &self.state.mmapper_ipc
    }
}
