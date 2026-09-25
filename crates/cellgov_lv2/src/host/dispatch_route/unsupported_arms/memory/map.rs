//! The `sys_mmapper` shared-memory map arms and the seed writes a first map carries.

use cellgov_effects::{Effect, WritePayload};
use cellgov_event::UnitId;
use cellgov_mem::ByteRange;
use cellgov_ps3_abi::lv2::errno;
use cellgov_time::GuestTicks;

use crate::dispatch::Lv2Dispatch;
use crate::host::mmapper::PendingRegionInstall;
use crate::host::Lv2Host;

impl Lv2Host {
    /// The IPC key a keyed 332 registered `mem_id` under, `None` for
    /// keyless handles. O(keyed handles); the map stays small.
    fn ipc_key_of_mem_id(&self, mem_id: u32) -> Option<u64> {
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
    /// against the 332/362 handle and the caller's committed layout,
    /// then pushes a pending region install.
    ///
    /// The install carries the handle's ipc key, so the runtime keeps
    /// every view of one keyed segment coherent. The window also enters
    /// the host ledger that sc 337's search consults. The first map of
    /// a seeded key co-emits its `SystemStateSeed` writes in the same
    /// effect batch. The occupancy test reads the caller's committed
    /// layout as well as that ledger, because the ledger alone cannot
    /// see regions the boot pipeline or the spawn loader installed.
    ///
    /// # Errors
    ///
    /// - `CELL_EINVAL` for `addr < 0x2000_0000 || addr >= 0xC000_0000`
    ///   or for `addr + handle.size` past `0xC000_0000`.
    /// - `CELL_ESRCH` (plus a `dispatch.mmapper_map_unknown_mem_id`
    ///   invariant break) when `mem_id` is not in the handle table.
    /// - `CELL_EALIGN` when `addr % handle.align != 0`.
    /// - `CELL_EBUSY` when the window intersects a region already
    ///   committed in the caller's space, or a window this host
    ///   already handed out through 334 / 337 -- loader images and
    ///   prior maps alike. Nothing public states that the kernel
    ///   refuses an overlapping claim rather than relocating it.
    ///
    /// `CELL_EINVAL` also answers a `mem_id` register that carries high
    /// bits, per [`Lv2Host::narrow_u32_args`]. The arm reads `addr` at
    /// its full width, so the window test above refuses a high `addr`.
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
        if !(0x2000_0000..0xC000_0000).contains(&addr) {
            return Lv2Dispatch::immediate(errno::CELL_EINVAL.into());
        }
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
        if !addr.is_multiple_of(u64::from(handle.align)) {
            return Lv2Dispatch::immediate(errno::CELL_EALIGN.into());
        }
        let Some(end) = addr.checked_add(u64::from(handle.size)) else {
            return Lv2Dispatch::immediate(errno::CELL_EINVAL.into());
        };
        if end > 0xC000_0000 {
            return Lv2Dispatch::immediate(errno::CELL_EINVAL.into());
        }
        // A window this host already handed out through 334 / 337 is
        // claimed even before the runtime commits its region, and the
        // ledger outlives an install the drain refused. Consulting it
        // alongside the caller's layout is what keeps ledger entries
        // non-overlapping, which `mmapper_ledger_insert`'s freshness
        // assertion depends on. The test is the furthest end over every
        // entry starting before this window: a shorter entry can start
        // later than -- and sit wholly inside -- a longer one, so
        // `next_back` alone would report a nested window as free.
        let ledger_busy = self
            .derived
            .mmapper_install_ledger
            .range(..(end as u32))
            .map(|(&start, &len)| u64::from(start) + u64::from(len))
            .max()
            .is_some_and(|entry_end| entry_end > addr);
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
                ipc_key: self.ipc_key_of_mem_id(mem_id),
            });
        // Record in the host ledger so sc 337's search sees this
        // range as occupied. `addr` is in `[0x2000_0000, 0xC000_0000)`
        // per the range check above, so the u32 narrow is lossless.
        self.mmapper_ledger_insert(addr as u32, handle.size);
        // Coherence witness: same property as sc 337.
        debug_assert!(
            self.derived
                .pending_region_installs
                .iter()
                .any(|i| i.addr == addr && i.size == handle.size as usize),
            "sc 334 coherence: pending_region_installs missing entry for {addr:#x}",
        );
        debug_assert!(
            self.derived
                .mmapper_install_ledger
                .contains_key(&(addr as u32)),
            "sc 334 coherence: mmapper_install_ledger missing entry for {addr:#x}",
        );
        self.note_system_ipc_map(mem_id, addr as u32, handle.size);
        let effects = self.system_seed_effects(mem_id, addr as u32, requester, tick);
        Lv2Dispatch::Immediate { code: 0, effects }
    }

    /// `sys_mmapper_search_and_map` (337): search for the first free
    /// aligned range of `handle.size` at or after `start_addr`
    /// inside the mmapper window, push a `PendingRegionInstall`,
    /// write the actual mapped address back to `*alloc_addr_ptr`.
    ///
    /// The interface carries `start_addr` in and `alloc_addr` out as
    /// separate arguments. The out-pointer receives the address the
    /// search settled on, not the caller's hint.
    ///
    /// An occupied window advances the candidate; the call does not
    /// fail on it. Occupied means present in the host install ledger
    /// or in the caller's committed layout. The kernel's search over
    /// the caller's VM area behaves the same way. A successful map
    /// records the window in the ledger and co-emits any registered
    /// seed for a keyed segment, as sc 334 does.
    ///
    /// # Errors
    /// - `CELL_EFAULT` when `alloc_addr_ptr` is null.
    /// - `CELL_EINVAL` when `start_addr` is outside the mmapper
    ///   window `[MMAPPER_REGION_START, MMAPPER_REGION_END)`.
    /// - `CELL_ESRCH` when `mem_id` is not in the handle table
    ///   (parallel to sc 334's `mmapper_map_unknown_mem_id` shape;
    ///   logs `dispatch.mmapper_search_and_map_unknown_mem_id`).
    /// - `CELL_ENOMEM` when the search exhausts the mmapper window.
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
        if let Some(d) = self.efault_if_null(&[alloc_addr_ptr]) {
            return d;
        }
        if !(Lv2Host::MMAPPER_REGION_START..Lv2Host::MMAPPER_REGION_END).contains(&start_addr) {
            return Lv2Dispatch::immediate(errno::CELL_EINVAL.into());
        }
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
        let Some(found_addr) =
            self.mmapper_search_free_range(start_addr, handle.size, handle.align, rt)
        else {
            return Lv2Dispatch::immediate(errno::CELL_ENOMEM.into());
        };
        self.derived
            .pending_region_installs
            .push(PendingRegionInstall {
                addr: u64::from(found_addr),
                size: handle.size as usize,
                ipc_key: self.ipc_key_of_mem_id(mem_id),
            });
        self.mmapper_ledger_insert(found_addr, handle.size);
        // Coherence witness: on success, the install must be pending
        // AND the ledger must contain the address we are writing
        // back.
        debug_assert!(
            self.derived
                .pending_region_installs
                .iter()
                .any(|i| i.addr == u64::from(found_addr) && i.size == handle.size as usize),
            "sc 337 coherence: pending_region_installs missing entry for {found_addr:#x}",
        );
        debug_assert!(
            self.derived
                .mmapper_install_ledger
                .contains_key(&found_addr),
            "sc 337 coherence: mmapper_install_ledger missing entry for {found_addr:#x}",
        );
        self.note_system_ipc_map(mem_id, found_addr, handle.size);
        let mut effects = self.system_seed_effects(mem_id, found_addr, requester, tick);
        effects.push(Effect::shared_write(
            ByteRange::contiguous_u32(alloc_addr_ptr, 4),
            WritePayload::from_slice(&found_addr.to_be_bytes()),
            requester,
            tick,
        ));
        Lv2Dispatch::Immediate { code: 0, effects }
    }
}
