//! `sys_mmapper_*` and `sys_memory_container_create` arms.

use cellgov_effects::{Effect, WritePayload};
use cellgov_event::UnitId;
use cellgov_mem::ByteRange;
use cellgov_ps3_abi::lv2::errno;
use cellgov_ps3_abi::lv2::memory::{
    ext_entry, page_size, CONTAINER_GRANULE, SYS_MMAPPER_NO_SHM_KEY, VM_AREA_ALIGNMENTS,
    VM_AREA_GRANULE,
};
use cellgov_time::GuestTicks;

use crate::dispatch::Lv2Dispatch;
use crate::host::guest_struct::read_be_u64;
use crate::host::mmapper::{MmapperHandle, PendingRegionInstall};
use crate::host::{Lv2Host, Lv2Runtime};

impl Lv2Host {
    /// `sys_memory_container_create`: mints a container id and writes
    /// it to `*cid`.
    ///
    /// Serves syscalls 324 and 341. Firmware publishes a wrapper for
    /// each: libsysmodule.sprx carries both in its syscall thunk
    /// table. CellGov routes them to one arm; whether the kernel binds
    /// them to one entry point is not witnessed here. Physical-memory
    /// budgets are not tracked.
    ///
    /// # Errors
    ///
    /// Listed in the order they fire.
    ///
    /// - `CELL_ENOMEM` when `size` rounds down to zero. A sub-granule
    ///   request has nothing left after truncation to the 1 MiB
    ///   granule. Where that refusal sits against the `cid` gate is
    ///   unestablished.
    /// - `CELL_EFAULT` when `cid` is null. The gate is CellGov's own;
    ///   no interface or firmware record establishes it.
    pub(in crate::host::dispatch_route) fn dispatch_memory_container_create(
        &mut self,
        cid_ptr: u32,
        size: u64,
        requester: UnitId,
        tick: GuestTicks,
    ) -> Lv2Dispatch {
        if size < CONTAINER_GRANULE {
            return Lv2Dispatch::immediate(errno::CELL_ENOMEM.into());
        }
        if let Some(d) = self.efault_if_null(&[cid_ptr]) {
            return d;
        }
        let cid = self.alloc_id();
        self.state.memory_containers.insert(cid);
        let write = Effect::shared_write(
            ByteRange::contiguous_u32(cid_ptr, 4),
            WritePayload::from_slice(&cid.to_be_bytes()),
            requester,
            tick,
        );
        Lv2Dispatch::Immediate {
            code: 0,
            effects: vec![write],
        }
    }

    /// `sys_mmapper_allocate_address` (330): bumps a 256 MiB-aligned
    /// cursor and writes its base to `*alloc_addr`.
    ///
    /// liblv2.sprx's own reservation asks for 256 MiB at an alignment
    /// of 256 MiB, so this cursor steps in the granule firmware uses.
    ///
    /// # Errors
    ///
    /// Listed in the order they fire. Where the argument gates sit
    /// against the `alloc_addr` gate is unestablished.
    ///
    /// - `CELL_EALIGN` when `size` is not a multiple of the 256 MiB
    ///   VM-area granule; a misaligned request is never rounded up.
    /// - `CELL_ENOMEM` when `size` does not fit in `u32`.
    /// - `CELL_EALIGN` when `alignment` is not one of the four area
    ///   sizes the kernel accepts.
    /// - `CELL_EFAULT` when `alloc_addr` is null. The gate is
    ///   CellGov's own; no interface or firmware record establishes
    ///   it.
    /// - `CELL_ENOMEM` when the VM window is exhausted.
    ///
    /// `CELL_EINVAL` also answers an `alloc_addr` register that carries
    /// high bits, per [`Lv2Host::narrow_u32_args`]. That gate precedes
    /// the list above.
    pub(in crate::host::dispatch_route) fn dispatch_mmapper_allocate_address(
        &mut self,
        args: [u64; 8],
        requester: UnitId,
        tick: GuestTicks,
    ) -> Lv2Dispatch {
        use cellgov_ps3_abi::lv2::syscall;

        let Some([alloc_addr_ptr]) =
            self.narrow_u32_args(syscall::MMAPPER_ALLOCATE_ADDRESS, [("alloc_addr", args[3])])
        else {
            return Lv2Dispatch::immediate(errno::CELL_EINVAL.into());
        };
        let size = args[0];
        let alignment = args[2];
        if !size.is_multiple_of(VM_AREA_GRANULE) {
            return Lv2Dispatch::immediate(errno::CELL_EALIGN.into());
        }
        let Ok(size) = u32::try_from(size) else {
            return Lv2Dispatch::immediate(errno::CELL_ENOMEM.into());
        };
        // A zero alignment is outside the accepted set, but it is
        // taken as the default area size. No firmware caller relies on
        // that: liblv2.sprx always names 256 MiB explicitly. The only
        // evidence for the allowance is PSL1GHT's sbrk, which depends
        // on it on real hardware.
        let alignment = if alignment == 0 {
            VM_AREA_GRANULE
        } else {
            alignment
        };
        if !VM_AREA_ALIGNMENTS.contains(&alignment) {
            return Lv2Dispatch::immediate(errno::CELL_EALIGN.into());
        }
        if let Some(d) = self.efault_if_null(&[alloc_addr_ptr]) {
            return d;
        }
        match self.mmapper_alloc(size) {
            Some(addr) => {
                let write = Effect::shared_write(
                    ByteRange::contiguous_u32(alloc_addr_ptr, 4),
                    WritePayload::from_slice(&addr.to_be_bytes()),
                    requester,
                    tick,
                );
                Lv2Dispatch::Immediate {
                    code: 0,
                    effects: vec![write],
                }
            }
            None => Lv2Dispatch::immediate(errno::CELL_ENOMEM.into()),
        }
    }

    /// `sys_mmapper_allocate_shared_memory` (332): mints a monotonic
    /// shm id and writes it to `*mem_id_ptr`.
    ///
    /// A non-sentinel, non-zero `ipc_key` (`args[0]`) routes through
    /// [`Lv2Host::mmapper_ipc`]: a registered key returns the
    /// existing `mem_id` with `size` / `flags` ignored, an
    /// unregistered key mints and registers.
    ///
    /// liblv2.sprx's `sys_mmapper_allocate_shared_memory` wrapper
    /// takes `(size, flags, mem_id)`, shifts them into args 1..=3, and
    /// loads the keyless sentinel into arg 0. Firmware therefore
    /// presents both the four-slot shape and the sentinel value.
    ///
    /// # Errors
    ///
    /// Listed in the order they fire. Where the argument gates sit
    /// against the `mem_id` gate is unestablished.
    ///
    /// - `CELL_EALIGN` when `size` is zero.
    /// - `CELL_EINVAL` when the `flags` granularity field carries an
    ///   encoding the kernel does not accept.
    /// - `CELL_ENOMEM` when `size` does not fit in `u32`.
    /// - `CELL_EALIGN` when `size` is not a multiple of the granule
    ///   the `flags` field selects.
    /// - `CELL_EFAULT` when `mem_id_ptr` is null. The gate is
    ///   CellGov's own; no interface or firmware record establishes
    ///   it.
    ///
    /// `CELL_EINVAL` also answers a `mem_id` register that carries high
    /// bits, per [`Lv2Host::narrow_u32_args`]. That gate precedes the
    /// list above.
    pub(in crate::host::dispatch_route) fn dispatch_mmapper_allocate_shared_memory(
        &mut self,
        args: [u64; 8],
        requester: UnitId,
        tick: GuestTicks,
    ) -> Lv2Dispatch {
        use cellgov_ps3_abi::lv2::syscall;

        let Some([mem_id_ptr]) = self.narrow_u32_args(
            syscall::MMAPPER_ALLOCATE_SHARED_MEMORY,
            [("mem_id", args[3])],
        ) else {
            return Lv2Dispatch::immediate(errno::CELL_EINVAL.into());
        };
        let ipc_key = args[0];
        let size = args[1];
        let flags = args[2];
        if size == 0 {
            return Lv2Dispatch::immediate(errno::CELL_EALIGN.into());
        }
        let Some(align) = accepted_granule(flags) else {
            return Lv2Dispatch::immediate(errno::CELL_EINVAL.into());
        };
        let Ok(size_u32) = u32::try_from(size) else {
            return Lv2Dispatch::immediate(errno::CELL_ENOMEM.into());
        };
        if !size_u32.is_multiple_of(align) {
            return Lv2Dispatch::immediate(errno::CELL_EALIGN.into());
        }
        if let Some(d) = self.efault_if_null(&[mem_id_ptr]) {
            return d;
        }
        let keyed = names_shared_segment(ipc_key);
        let in_namespace = keyed && crate::host::is_system_ipc_key(ipc_key);
        let mem_id = match self.state.mmapper_ipc.get(&ipc_key) {
            Some(&existing) if keyed => {
                if in_namespace {
                    self.obs.system_ipc_witness.shm_attaches += 1;
                    self.obs.system_ipc_witness.note_key(ipc_key);
                }
                existing
            }
            _ => {
                let mem_id = self.alloc_id();
                self.state.mmapper_handles.insert(
                    mem_id,
                    MmapperHandle {
                        size: size_u32,
                        align,
                    },
                );
                if keyed {
                    self.state.mmapper_ipc.insert(ipc_key, mem_id);
                }
                if in_namespace {
                    self.obs.system_ipc_witness.shm_creates += 1;
                    self.obs.system_ipc_witness.note_key(ipc_key);
                }
                mem_id
            }
        };
        let write = Effect::shared_write(
            ByteRange::contiguous_u32(mem_id_ptr, 4),
            WritePayload::from_slice(&mem_id.to_be_bytes()),
            requester,
            tick,
        );
        Lv2Dispatch::Immediate {
            code: 0,
            effects: vec![write],
        }
    }

    /// The IPC key a keyed 332 registered `mem_id` under, `None` for
    /// keyless handles. O(keyed handles); the map stays small.
    fn ipc_key_of_mem_id(&self, mem_id: u32) -> Option<u64> {
        self.state
            .mmapper_ipc
            .iter()
            .find(|&(_, &id)| id == mem_id)
            .map(|(&k, _)| k)
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

    /// `sys_mmapper_allocate_shared_memory_from_container` (362):
    /// container variant of 332, with flags at r6 (`args[3]`) and
    /// `mem_id` out-pointer at r7.
    ///
    /// # Errors
    ///
    /// Same set and same order as
    /// `dispatch_mmapper_allocate_shared_memory`, including the
    /// `mem_id` width gate that precedes them.
    pub(in crate::host::dispatch_route) fn dispatch_mmapper_allocate_shared_memory_from_container(
        &mut self,
        args: [u64; 8],
        requester: UnitId,
        tick: GuestTicks,
    ) -> Lv2Dispatch {
        use cellgov_ps3_abi::lv2::syscall;

        let Some([mem_id_ptr]) = self.narrow_u32_args(
            syscall::MMAPPER_ALLOCATE_SHARED_MEMORY_FROM_CONTAINER,
            [("mem_id", args[4])],
        ) else {
            return Lv2Dispatch::immediate(errno::CELL_EINVAL.into());
        };
        let size = args[1];
        let flags = args[3];
        if size == 0 {
            return Lv2Dispatch::immediate(errno::CELL_EALIGN.into());
        }
        let Some(align) = accepted_granule(flags) else {
            return Lv2Dispatch::immediate(errno::CELL_EINVAL.into());
        };
        let Ok(size_u32) = u32::try_from(size) else {
            return Lv2Dispatch::immediate(errno::CELL_ENOMEM.into());
        };
        if !size_u32.is_multiple_of(align) {
            return Lv2Dispatch::immediate(errno::CELL_EALIGN.into());
        }
        if let Some(d) = self.efault_if_null(&[mem_id_ptr]) {
            return d;
        }
        let mem_id = self.alloc_id();
        self.state.mmapper_handles.insert(
            mem_id,
            MmapperHandle {
                size: size_u32,
                align,
            },
        );
        let write = Effect::shared_write(
            ByteRange::contiguous_u32(mem_id_ptr, 4),
            WritePayload::from_slice(&mem_id.to_be_bytes()),
            requester,
            tick,
        );
        Lv2Dispatch::Immediate {
            code: 0,
            effects: vec![write],
        }
    }

    /// `sys_mmapper_allocate_shared_memory_ext` (339): exclusive keyed
    /// variant of 332 with a per-entry attribute table at r6/r7 and
    /// the `mem_id` out-pointer at r8.
    ///
    /// A key already registered answers `CELL_EEXIST` where 332 would
    /// attach; callers probe a key range on that answer. Zero and
    /// `SYS_MMAPPER_NO_SHM_KEY` are not keys, so they register nothing
    /// and every keyless call mints a fresh id. Eleven installed modules
    /// issue 339 across fourteen sites -- twelve inline calls plus two
    /// exported wrappers -- so firmware does exercise the call. Those
    /// sites fix the argument shape: the ones that build the word
    /// inline pass the 64 KiB granularity flag in r5, one or two
    /// entries in r7, and an out-pointer in r8. None of them fixes
    /// what the kernel answers for a key it already holds, so the
    /// exclusive-create rule is CellGov's.
    ///
    /// # Errors
    ///
    /// Listed in the order they fire.
    ///
    /// - `CELL_EALIGN` when `size` is zero.
    /// - `CELL_EINVAL` when the `flags` granularity field carries an
    ///   encoding the kernel does not accept.
    /// - `CELL_ENOMEM` when `size` does not fit in `u32`.
    /// - `CELL_EALIGN` when `size` is not a multiple of the granule.
    /// - `CELL_EINVAL` when `flags` carries bits outside the
    ///   granularity field.
    /// - `CELL_EINVAL` when `entry_count` is outside `1..=16`.
    ///   `entry_count` is an `int`, and the arm reads only the low word
    ///   of its register. The range test runs on that word, so the arm
    ///   drops a high word. The `entries` and `mem_id` gates below
    ///   refuse a high word. Which answer the kernel gives either
    ///   field is unestablished.
    /// - `CELL_EFAULT` when an entry's `type` word is unreadable.
    /// - `CELL_EPERM` when an entry type is unknown, or privileged
    ///   without 64 KiB pages and debug-or-root capability.
    /// - `CELL_EFAULT` when `mem_id_ptr` is null.
    /// - `CELL_EEXIST` when a keyed `ipc_key` is already registered.
    ///
    /// `CELL_EINVAL` also answers a `flags`, `entries` or `mem_id`
    /// register that carries high bits, per
    /// [`Lv2Host::narrow_u32_args`]. That gate precedes the list above.
    /// This arm's `flags` is a 32-bit word, unlike the 64-bit one 332
    /// and 362 take.
    pub(in crate::host::dispatch_route) fn dispatch_mmapper_allocate_shared_memory_ext(
        &mut self,
        args: [u64; 8],
        requester: UnitId,
        rt: &dyn Lv2Runtime,
        tick: GuestTicks,
    ) -> Lv2Dispatch {
        use cellgov_ps3_abi::lv2::syscall;

        let Some([flags, entries_ptr, mem_id_ptr]) = self.narrow_u32_args(
            syscall::MMAPPER_ALLOCATE_SHARED_MEMORY_EXT,
            [
                ("flags", args[2]),
                ("entries", args[3]),
                ("mem_id", args[5]),
            ],
        ) else {
            return Lv2Dispatch::immediate(errno::CELL_EINVAL.into());
        };
        let flags = u64::from(flags);
        let ipc_key = args[0];
        let size = args[1];
        let entry_count = args[4] as i32;
        if size == 0 {
            return Lv2Dispatch::immediate(errno::CELL_EALIGN.into());
        }
        let Some(align) = accepted_granule(flags) else {
            return Lv2Dispatch::immediate(errno::CELL_EINVAL.into());
        };
        let Ok(size_u32) = u32::try_from(size) else {
            return Lv2Dispatch::immediate(errno::CELL_ENOMEM.into());
        };
        if !size_u32.is_multiple_of(align) {
            return Lv2Dispatch::immediate(errno::CELL_EALIGN.into());
        }
        if flags & !page_size::GRANULARITY_FIELD != 0 {
            return Lv2Dispatch::immediate(errno::CELL_EINVAL.into());
        }
        if entry_count <= 0 || entry_count > ext_entry::MAX_COUNT {
            return Lv2Dispatch::immediate(errno::CELL_EINVAL.into());
        }
        for i in 0..entry_count as u32 {
            let Some(type_addr) = entries_ptr
                .checked_add(i * ext_entry::LEN)
                .and_then(|e| e.checked_add(ext_entry::TYPE_OFFSET))
            else {
                return Lv2Dispatch::immediate(errno::CELL_EFAULT.into());
            };
            let Some(entry_type) = read_be_u64(rt, u64::from(type_addr)) else {
                return Lv2Dispatch::immediate(errno::CELL_EFAULT.into());
            };
            let admitted = ext_entry::PLAIN_TYPES.contains(&entry_type)
                || (entry_type == ext_entry::PRIVILEGED_TYPE
                    && flags == page_size::FLAG_64K
                    && self.debug_or_root());
            if !admitted {
                return Lv2Dispatch::immediate(errno::CELL_EPERM.into());
            }
        }
        if let Some(d) = self.efault_if_null(&[mem_id_ptr]) {
            return d;
        }
        let keyed = names_shared_segment(ipc_key);
        if keyed && self.state.mmapper_ipc.contains_key(&ipc_key) {
            return Lv2Dispatch::immediate(errno::CELL_EEXIST.into());
        }
        let mem_id = self.alloc_id();
        self.state.mmapper_handles.insert(
            mem_id,
            MmapperHandle {
                size: size_u32,
                align,
            },
        );
        if keyed {
            self.state.mmapper_ipc.insert(ipc_key, mem_id);
            if crate::host::is_system_ipc_key(ipc_key) {
                self.obs.system_ipc_witness.shm_creates += 1;
                self.obs.system_ipc_witness.note_key(ipc_key);
            }
        }
        let write = Effect::shared_write(
            ByteRange::contiguous_u32(mem_id_ptr, 4),
            WritePayload::from_slice(&mem_id.to_be_bytes()),
            requester,
            tick,
        );
        Lv2Dispatch::Immediate {
            code: 0,
            effects: vec![write],
        }
    }
}

/// Whether an `ipc_key` names a process-shared segment.
///
/// 332 and 339 read this argument the same way: a key that names no
/// segment registers nothing, so it cannot collide with itself.
/// Whether zero belongs on the keyless side is unestablished.
/// `SYS_MMAPPER_NO_SHM_KEY` is the sentinel the interface names; a
/// zero key reads as an unset field, which is CellGov's own reading.
fn names_shared_segment(ipc_key: u64) -> bool {
    ipc_key != 0 && ipc_key != SYS_MMAPPER_NO_SHM_KEY
}

/// The page granule a `sys_mmapper` `flags` word selects, `None` when
/// its granularity field holds an encoding the kernel refuses.
///
/// The granularity field is `flags` bits [8,11]. 64 KiB and 1 MiB are
/// the only encodings it defines, and an unset field takes the
/// default. liblv2.sprx's own 256 MiB reservation carries the 64 KiB
/// encoding in exactly those bits. Anything else is refused rather
/// than rounded to a granule; nothing public establishes the refusal
/// itself.
fn accepted_granule(flags: u64) -> Option<u32> {
    match flags & page_size::GRANULARITY_FIELD {
        0 | page_size::FLAG_64K | page_size::FLAG_1M => Some(page_size::granule_from_flags(flags)),
        _ => None,
    }
}
