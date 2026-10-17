//! The `sys_mmapper` shared-memory map arms and the seed writes a first map carries.

use cellgov_effects::{Effect, WritePayload};
use cellgov_event::UnitId;
use cellgov_mem::ByteRange;
use cellgov_ps3_abi::lv2::errno;
use cellgov_time::GuestTicks;

use crate::dispatch::Lv2Dispatch;
use crate::host::mmapper::{PendingRegionInstall, PendingRegionRemoval};
use crate::host::Lv2Host;

impl Lv2Host {
    /// The IPC key a keyed 332 registered `mem_id` under, `None` for
    /// keyless handles. O(keyed handles); the map stays small.
    pub(in crate::host) fn ipc_key_of_mem_id(&self, mem_id: u32) -> Option<u64> {
        self.state
            .mmapper_ipc
            .iter()
            .find(|&(_, &id)| id == mem_id)
            .map(|(k, _)| k)
    }

    /// Seed effects for the first map of an ipc-keyed shm with a
    /// registered [`crate::SystemStateSeed`]; empty for keyless shms,
    /// unseeded keys, and already-applied seeds.
    ///
    /// # Cross-module contract
    ///
    /// The caller (334 / 337) must co-emit these in the same
    /// `Lv2Dispatch::Immediate` as its own writes so the commit
    /// pipeline applies the seed atomically with the map: the shm is
    /// guest-observable from that dispatch's return forward, so the
    /// seed must be present by then.
    fn system_seed_effects(
        &mut self,
        mem_id: u32,
        base_addr: u32,
        requester: UnitId,
        tick: GuestTicks,
    ) -> Vec<Effect> {
        let Some(ipc_key) = self.ipc_key_of_mem_id(mem_id) else {
            return Vec::new();
        };
        if self.derived.system_seeds_applied.contains(&ipc_key) {
            return Vec::new();
        }
        let Some(seed) = self.derived.system_state_seeds.get(&ipc_key) else {
            return Vec::new();
        };
        self.derived.system_seeds_applied.insert(ipc_key);
        self.derived.system_seed_bases.insert(ipc_key, base_addr);
        let handle_size = self
            .state
            .mmapper_handles
            .get(mem_id)
            .map(|h| h.size)
            .unwrap_or(0);
        seed.writes
            .iter()
            .map(|(offset, bytes)| {
                debug_assert!(
                    u64::from(*offset) + bytes.len() as u64 <= u64::from(handle_size),
                    "seed write at +{offset:#x} ({} bytes) exceeds shm size {handle_size:#x}",
                    bytes.len(),
                );
                Effect::shared_write(
                    ByteRange::contiguous_u32(base_addr.wrapping_add(*offset), bytes.len() as u32),
                    WritePayload::from_slice(bytes),
                    requester,
                    tick,
                )
            })
            .collect()
    }

    /// `sys_mmapper_map_shared_memory` (334): validates `addr`
    /// against the area the caller's 330 reserved around it, the 332/362 handle and
    /// the caller's committed layout, then pushes a pending region
    /// install.
    ///
    /// The install carries the handle's ipc key, so the runtime keeps
    /// every view of one keyed segment coherent. The window also enters
    /// the host window table that sc 337's search and sc 335 consult. The first map of
    /// a seeded key co-emits its `SystemStateSeed` writes in the same
    /// effect batch. The occupancy test reads the caller's committed
    /// layout as well as that table, because the table alone cannot
    /// see regions the boot pipeline or the spawn loader installed.
    ///
    /// # Errors
    ///
    /// Listed in the order they fire, as the `lv2_mmapper_map` (an area
    /// of 1 MiB pages) and `lv2_mmapper_map_64k` captures show. `flags`
    /// is not checked: both captures map under 0x999 and read-only.
    ///
    /// - `CELL_EINVAL` when no area the caller's 330 reserved holds
    ///   `addr`.
    /// - `CELL_ESRCH` (plus a `dispatch.mmapper_map_unknown_mem_id`
    ///   invariant break) when `mem_id` is not in the handle table.
    /// - `CELL_EINVAL` when the segment's granule is finer than the
    ///   area's page size.
    /// - `CELL_EALIGN` when `addr` is not a multiple of the area's page
    ///   size. A segment of 1 MiB granule maps on a 64 KiB boundary of
    ///   an area of 64 KiB pages.
    /// - `CELL_EINVAL` when the segment runs past the end of the area,
    ///   as the `lv2_mmapper_map_edges` capture shows.
    /// - `CELL_EBUSY` when the window intersects a region already
    ///   committed in the caller's space, or a window the caller's
    ///   334 / 337 already mapped -- loader images and
    ///   prior maps alike. Nothing public states that the kernel
    ///   refuses an overlapping claim rather than relocating it.
    ///
    /// `CELL_EINVAL` also answers a `mem_id` register that carries high
    /// bits, per [`Lv2Host::narrow_u32_args`]. The arm reads `addr` at
    /// its full width, so the area test above refuses a high `addr`.
    pub(in crate::host::dispatch_route) fn dispatch_mmapper_map_shared_memory(
        &mut self,
        args: [u64; 8],
        requester: UnitId,
        rt: &dyn crate::host::Lv2Runtime,
        tick: GuestTicks,
    ) -> Lv2Dispatch {
        use cellgov_ps3_abi::lv2::syscall;

        let Some([mem_id]) =
            self.narrow_u32_args(syscall::MMAPPER_MAP_SHARED_MEMORY, [("mem_id", args[1])])
        else {
            return Lv2Dispatch::immediate(errno::CELL_EINVAL.into());
        };
        let addr = args[0];
        let pid = self.state.processes.process_of_unit(requester);
        let Some((area_base, area)) = self.mmapper_area_of(pid, addr) else {
            return Lv2Dispatch::immediate(errno::CELL_EINVAL.into());
        };
        let Some(handle) = self.state.mmapper_handles.get(mem_id) else {
            self.log_invariant_break(
                "dispatch.mmapper_map_unknown_mem_id",
                format_args!(
                    "sys_mmapper_map_shared_memory mem_id={mem_id:#x} not in handle table; \
                     332 / 362 must precede 334"
                ),
            );
            return Lv2Dispatch::immediate(errno::CELL_ESRCH.into());
        };
        if handle.align < area.page {
            return Lv2Dispatch::immediate(errno::CELL_EINVAL.into());
        }
        if !addr.is_multiple_of(u64::from(area.page)) {
            return Lv2Dispatch::immediate(errno::CELL_EALIGN.into());
        }
        let end = addr + u64::from(handle.size);
        if end > u64::from(area_base) + u64::from(area.size) {
            return Lv2Dispatch::immediate(errno::CELL_EINVAL.into());
        }
        // A window the caller's 334 / 337 already mapped is
        // claimed even before the runtime commits its region, and the
        // window outlives an install the drain refused. Consulting the
        // window table alongside the caller's layout is what keeps
        // windows non-overlapping, which `mmapper_window_insert`'s
        // freshness assertion depends on. The test is the furthest end
        // over every window starting before this one: a shorter window
        // can start later than -- and sit wholly inside -- a longer one,
        // so the nearest window alone would report a nested one as free.
        let ledger_busy = self
            .mmapper_window_ends_below(pid, end as u32)
            .max()
            .is_some_and(|window_end| u64::from(window_end) > addr);
        if ledger_busy
            || rt
                .committed_overlap_end(addr, u64::from(handle.size))
                .is_some()
        {
            return Lv2Dispatch::immediate(errno::CELL_EBUSY.into());
        }
        self.derived
            .pending_region_installs
            .push(PendingRegionInstall {
                addr,
                size: handle.size as usize,
                mem_id,
                ipc_key: self.ipc_key_of_mem_id(mem_id),
            });
        // Record the window so sc 337's search sees this range as
        // occupied and sc 335 finds it. `addr` lies inside a 330 area
        // per the area test above, so the u32 narrow is lossless.
        self.mmapper_window_insert(pid, addr as u32, handle.size, mem_id);
        // Coherence witness: same property as sc 337.
        debug_assert!(
            self.derived
                .pending_region_installs
                .iter()
                .any(|i| i.addr == addr && i.size == handle.size as usize),
            "sc 334 coherence: pending_region_installs missing entry for {addr:#x}",
        );
        debug_assert!(
            self.state.mmapper_windows.contains_key((pid, addr as u32)),
            "sc 334 coherence: mmapper_windows missing entry for {addr:#x}",
        );
        self.note_system_ipc_map(pid, mem_id, addr as u32, handle.size);
        let effects = self.system_seed_effects(mem_id, addr as u32, requester, tick);
        Lv2Dispatch::Immediate { code: 0, effects }
    }

    /// `sys_mmapper_search_and_map` (337): search the area the
    /// caller's 330 reserved at `start_addr` for the first free range of
    /// `handle.size` on the area's page size, push a
    /// `PendingRegionInstall`, and write the mapped address back to
    /// `*alloc_addr_ptr`.
    ///
    /// The interface carries `start_addr` in and `alloc_addr` out as
    /// separate arguments. The out-pointer receives the address the
    /// search settled on, not the caller's hint.
    ///
    /// An occupied window advances the candidate; the call does not
    /// fail on it. Occupied means a window of the caller's in the
    /// window table, or a range in the caller's committed layout. The kernel's search over
    /// the caller's VM area behaves the same way. A successful map
    /// records the window in the window table and co-emits any registered
    /// seed for a keyed segment, as sc 334 does.
    ///
    /// # Errors
    ///
    /// Listed in the order they fire, as the `lv2_mmapper_map` and
    /// `lv2_mmapper_map_64k` captures show.
    ///
    /// - `CELL_EINVAL` when no area the caller's 330 reserved holds
    ///   `start_addr`.
    /// - `CELL_ESRCH` when `mem_id` is not in the handle table
    ///   (parallel to sc 334's `mmapper_map_unknown_mem_id` shape;
    ///   logs `dispatch.mmapper_search_and_map_unknown_mem_id`).
    /// - `CELL_EALIGN` when the segment's granule is finer than the
    ///   area's page size.
    /// - `CELL_EINVAL` when `start_addr` is inside an area but not its
    ///   base.
    /// - `CELL_EFAULT` when `alloc_addr_ptr` is null. The segment stays
    ///   busy against its free for good, as the `lv2_mmapper_unmap`
    ///   capture shows; the area does not.
    /// - `CELL_ENOMEM` when the area has no free range for the segment.
    /// - `CELL_EINVAL` when `start_addr`, `mem_id` or `alloc_addr`
    ///   carries high bits, per [`Lv2Host::narrow_u32_args`]. That gate
    ///   precedes the rest.
    pub(in crate::host::dispatch_route) fn dispatch_mmapper_search_and_map(
        &mut self,
        args: [u64; 8],
        requester: UnitId,
        rt: &dyn crate::host::Lv2Runtime,
        tick: GuestTicks,
    ) -> Lv2Dispatch {
        use cellgov_ps3_abi::lv2::syscall;

        let Some([start_addr, mem_id, alloc_addr_ptr]) = self.narrow_u32_args(
            syscall::MMAPPER_SEARCH_AND_MAP,
            [
                ("start_addr", args[0]),
                ("mem_id", args[1]),
                ("alloc_addr", args[3]),
            ],
        ) else {
            return Lv2Dispatch::immediate(errno::CELL_EINVAL.into());
        };
        let pid = self.state.processes.process_of_unit(requester);
        let Some((area_base, area)) = self.mmapper_area_of(pid, u64::from(start_addr)) else {
            return Lv2Dispatch::immediate(errno::CELL_EINVAL.into());
        };
        let Some(handle) = self.state.mmapper_handles.get(mem_id) else {
            self.log_invariant_break(
                "dispatch.mmapper_search_and_map_unknown_mem_id",
                format_args!(
                    "sys_mmapper_search_and_map mem_id={mem_id:#x} not in handle table; \
                     332 / 362 must precede 337"
                ),
            );
            return Lv2Dispatch::immediate(errno::CELL_ESRCH.into());
        };
        if handle.align < area.page {
            return Lv2Dispatch::immediate(errno::CELL_EALIGN.into());
        }
        if start_addr != area_base {
            return Lv2Dispatch::immediate(errno::CELL_EINVAL.into());
        }
        if let Some(d) = self.efault_if_null(&[alloc_addr_ptr]) {
            // The `lv2_mmapper_map_64k` and `lv2_mmapper_unmap` captures
            // show this refusal leaving no window, yet the segment answers
            // CELL_EBUSY to every later free while its area frees: the
            // segment's map is counted before the address write faults,
            // and never uncounted.
            self.state.mmapper_handles.note_stuck_map(mem_id);
            return d;
        }
        let limit = start_addr.saturating_add(area.size);
        // The `lv2_mmapper_map_edges` capture places a 1 MiB segment
        // 64 KiB above a 64 KiB map in an area of 64 KiB pages: the
        // search steps on the area's page size, not the segment's
        // granule.
        let Some(found_addr) =
            self.mmapper_search_free_range(pid, start_addr, handle.size, area.page, limit, rt)
        else {
            return Lv2Dispatch::immediate(errno::CELL_ENOMEM.into());
        };
        self.derived
            .pending_region_installs
            .push(PendingRegionInstall {
                addr: u64::from(found_addr),
                size: handle.size as usize,
                mem_id,
                ipc_key: self.ipc_key_of_mem_id(mem_id),
            });
        self.mmapper_window_insert(pid, found_addr, handle.size, mem_id);
        // Coherence witness: on success, the install must be pending
        // AND the window table must contain the address we are writing
        // back.
        debug_assert!(
            self.derived
                .pending_region_installs
                .iter()
                .any(|i| i.addr == u64::from(found_addr) && i.size == handle.size as usize),
            "sc 337 coherence: pending_region_installs missing entry for {found_addr:#x}",
        );
        debug_assert!(
            self.state.mmapper_windows.contains_key((pid, found_addr)),
            "sc 337 coherence: mmapper_windows missing entry for {found_addr:#x}",
        );
        self.note_system_ipc_map(pid, mem_id, found_addr, handle.size);
        let mut effects = self.system_seed_effects(mem_id, found_addr, requester, tick);
        effects.push(Effect::shared_write(
            ByteRange::contiguous_u32(alloc_addr_ptr, 4),
            WritePayload::from_slice(&found_addr.to_be_bytes()),
            requester,
            tick,
        ));
        Lv2Dispatch::Immediate { code: 0, effects }
    }

    /// `sys_mmapper_unmap_shared_memory` (335): removes the window 334
    /// or 337 mapped at `addr`, pushes a pending region removal, and
    /// writes the segment's id to `*mem_id_ptr`.
    ///
    /// The `lv2_mmapper_unmap` capture shows 329 refusing the free of a
    /// segment mapped at two windows until 335 unmaps both.
    ///
    /// # Errors
    ///
    /// Listed in the order they fire, as the `lv2_mmapper_map_64k` and
    /// `lv2_mmapper_unmap` captures show. A refusal writes nothing.
    ///
    /// - `CELL_EINVAL` when no window of the caller's starts at `addr`
    ///   (another process's window there included): an address
    ///   inside a window, an address outside every area, and a window
    ///   already unmapped alike, with a null `mem_id_ptr` too.
    /// - `CELL_EFAULT` when `mem_id_ptr` is null. The window stays
    ///   mapped.
    ///
    /// `CELL_EINVAL` also answers an `addr` or `mem_id_ptr` register
    /// that carries high bits, per [`Lv2Host::narrow_u32_args`]. That
    /// gate precedes the list above.
    pub(in crate::host::dispatch_route) fn dispatch_mmapper_unmap_shared_memory(
        &mut self,
        args: [u64; 8],
        requester: UnitId,
        tick: GuestTicks,
    ) -> Lv2Dispatch {
        use cellgov_ps3_abi::lv2::syscall;

        let Some([addr, mem_id_ptr]) = self.narrow_u32_args(
            syscall::MMAPPER_UNMAP_SHARED_MEMORY,
            [("addr", args[0]), ("mem_id", args[1])],
        ) else {
            return Lv2Dispatch::immediate(errno::CELL_EINVAL.into());
        };
        let pid = self.state.processes.process_of_unit(requester);
        let Some(&window) = self.state.mmapper_windows.get((pid, addr)) else {
            return Lv2Dispatch::immediate(errno::CELL_EINVAL.into());
        };
        if let Some(d) = self.efault_if_null(&[mem_id_ptr]) {
            return d;
        }
        self.state.mmapper_windows.remove((pid, addr));
        // The shm-write witness counts writes into mapped ranges only.
        self.obs.system_ipc_mappings.remove(&(pid, addr));
        self.derived
            .pending_region_removals
            .push(PendingRegionRemoval {
                addr: u64::from(addr),
                size: window.size as usize,
                mem_id: window.mem_id,
                ipc_key: self.ipc_key_of_mem_id(window.mem_id),
            });
        let write = Effect::shared_write(
            ByteRange::contiguous_u32(mem_id_ptr, 4),
            WritePayload::from_slice(&window.mem_id.to_be_bytes()),
            requester,
            tick,
        );
        Lv2Dispatch::Immediate {
            code: 0,
            effects: vec![write],
        }
    }
}
