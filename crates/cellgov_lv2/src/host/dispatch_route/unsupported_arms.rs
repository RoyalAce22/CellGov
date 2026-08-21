//! Per-syscall dispatch helpers for the `Lv2Request::Unsupported
//! { number: N }` arms; one method per syscall number.

use cellgov_effects::{Effect, WritePayload};
use cellgov_event::{PriorityClass, UnitId};
use cellgov_mem::ByteRange;
use cellgov_ps3_abi::cell_errors;
use cellgov_ps3_abi::sys_memory::{
    page_size, CONTAINER_GRANULE, SYS_MMAPPER_NO_SHM_KEY, VM_AREA_ALIGNMENTS, VM_AREA_GRANULE,
};

use crate::dispatch::Lv2Dispatch;

use crate::host::mmapper::{MmapperHandle, PendingRegionInstall};
use crate::host::{Lv2Host, Lv2Runtime};
use cellgov_time::GuestTicks;

impl Lv2Host {
    /// `sys_ppu_thread_get_priority` (48): writes the target's
    /// priority to `*priop`, CELL_ESRCH for ids absent from the
    /// thread table.
    ///
    /// The id lookup precedes the `priop` null gate, matching
    /// RPCS3's lookup-then-write order. Oracle: RPCS3's
    /// `sys_ppu_thread.cpp`.
    pub(super) fn dispatch_ppu_thread_get_priority(
        &self,
        args: [u64; 8],
        requester: UnitId,
        tick: GuestTicks,
    ) -> Lv2Dispatch {
        let thread_id = args[0] as u32;
        let priop = args[1] as u32;
        let Some(thread) = self
            .state
            .ppu_threads
            .get(crate::ppu_thread::PpuThreadId::new(thread_id as u64))
        else {
            return Lv2Dispatch::immediate(cell_errors::CELL_ESRCH.into());
        };
        if let Some(d) = self.efault_if_null(&[priop]) {
            return d;
        }
        let write = Effect::SharedWriteIntent {
            range: ByteRange::contiguous_u32(priop, 4),
            bytes: WritePayload::from_slice(&thread.attrs.priority.to_be_bytes()),
            ordering: PriorityClass::Normal,
            source: requester,
            source_time: tick,
        };
        Lv2Dispatch::Immediate {
            code: 0,
            effects: vec![write],
        }
    }

    /// `sys_memory_container_create`: mints a container id and writes
    /// it to `*cid`.
    ///
    /// Serves syscalls 324 and 341, which RPCS3's syscall table binds
    /// to this one kernel entry point, so both answer identically.
    ///
    /// Physical-memory budgets are not tracked, so the only refusal
    /// modelled from the container size is the one that does not need
    /// a budget. Oracle: RPCS3's `sys_memory.cpp`.
    ///
    /// # Errors
    ///
    /// Listed in the order they fire, which is the order RPCS3 checks
    /// them.
    ///
    /// - `CELL_ENOMEM` when `size` rounds down to zero. The kernel
    ///   truncates the request to the 1 MiB granule and refuses what
    ///   is left of a sub-granule request (RPCS3
    ///   `sys_memory_container_create`).
    /// - `CELL_EFAULT` when `cid` is null. RPCS3 has no such gate --
    ///   it writes through the pointer once the container exists --
    ///   so the gate sits after the size refusal and before the id is
    ///   minted.
    pub(super) fn dispatch_memory_container_create(
        &mut self,
        cid_ptr: u32,
        size: u64,
        requester: UnitId,
        tick: GuestTicks,
    ) -> Lv2Dispatch {
        if size < CONTAINER_GRANULE {
            return Lv2Dispatch::immediate(cell_errors::CELL_ENOMEM.into());
        }
        if let Some(d) = self.efault_if_null(&[cid_ptr]) {
            return d;
        }
        let cid = self.alloc_id();
        let write = Effect::SharedWriteIntent {
            range: ByteRange::contiguous_u32(cid_ptr, 4),
            bytes: WritePayload::from_slice(&cid.to_be_bytes()),
            ordering: PriorityClass::Normal,
            source: requester,
            source_time: tick,
        };
        Lv2Dispatch::Immediate {
            code: 0,
            effects: vec![write],
        }
    }

    /// `sys_mmapper_allocate_address` (330): bumps a 256 MiB-aligned
    /// cursor and writes its base to `*alloc_addr`.
    ///
    /// Oracle: RPCS3's `sys_mmapper.cpp`
    /// `sys_mmapper_allocate_address`.
    ///
    /// # Errors
    ///
    /// Listed in the order they fire, which is the order RPCS3 checks
    /// them: the argument gates precede the `alloc_addr` gate, so a
    /// call that is wrong in two ways answers for its `size` or
    /// `alignment` first.
    ///
    /// - `CELL_EALIGN` when `size` is not a multiple of the 256 MiB
    ///   VM-area granule. A misaligned request is refused, never
    ///   rounded up to the next granule.
    /// - `CELL_ENOMEM` when `size` does not fit in `u32`.
    /// - `CELL_EALIGN` when `alignment` is not one of the four area
    ///   sizes the kernel accepts.
    /// - `CELL_EFAULT` when `alloc_addr` is null. RPCS3 has no such
    ///   gate -- it reserves the area and then writes through the
    ///   pointer -- so the gate sits as late as it can without
    ///   consuming a VM area the caller can never read back.
    /// - `CELL_ENOMEM` when the VM window is exhausted.
    pub(super) fn dispatch_mmapper_allocate_address(
        &mut self,
        args: [u64; 8],
        requester: UnitId,
        tick: GuestTicks,
    ) -> Lv2Dispatch {
        let size = args[0];
        let alignment = args[2];
        let alloc_addr_ptr = args[3] as u32;
        if !size.is_multiple_of(VM_AREA_GRANULE) {
            return Lv2Dispatch::immediate(cell_errors::CELL_EALIGN.into());
        }
        let Ok(size) = u32::try_from(size) else {
            return Lv2Dispatch::immediate(cell_errors::CELL_ENOMEM.into());
        };
        // A zero alignment is technically invalid but the hardware
        // accepts it as the default area size; PSL1GHT's sbrk relies
        // on that, and RPCS3 carries the same allowance.
        let alignment = if alignment == 0 {
            VM_AREA_GRANULE
        } else {
            alignment
        };
        if !VM_AREA_ALIGNMENTS.contains(&alignment) {
            return Lv2Dispatch::immediate(cell_errors::CELL_EALIGN.into());
        }
        if let Some(d) = self.efault_if_null(&[alloc_addr_ptr]) {
            return d;
        }
        match self.mmapper_alloc(size) {
            Some(addr) => {
                let write = Effect::SharedWriteIntent {
                    range: ByteRange::contiguous_u32(alloc_addr_ptr, 4),
                    bytes: WritePayload::from_slice(&addr.to_be_bytes()),
                    ordering: PriorityClass::Normal,
                    source: requester,
                    source_time: tick,
                };
                Lv2Dispatch::Immediate {
                    code: 0,
                    effects: vec![write],
                }
            }
            None => Lv2Dispatch::immediate(cell_errors::CELL_ENOMEM.into()),
        }
    }

    /// `sys_mmapper_allocate_shared_memory` (332): mints a monotonic
    /// shm id and writes it to `*mem_id_ptr`.
    ///
    /// A non-sentinel, non-zero `ipc_key` (`args[0]`) routes through
    /// [`Lv2Host::mmapper_ipc`]: a registered key returns the
    /// existing `mem_id` with `size` / `flags` ignored, an
    /// unregistered key mints and registers. Oracle: RPCS3's
    /// `create_lv2_shm` SYS_SYNC_NOT_CARE path
    /// (`sys_mmapper.cpp`).
    ///
    /// # Errors
    ///
    /// Listed in the order they fire. The argument gates precede the
    /// `mem_id` gate, matching RPCS3's order; a call that is wrong in
    /// two ways answers for its `size` or `flags` first.
    ///
    /// - `CELL_EALIGN` when `size` is zero.
    /// - `CELL_EINVAL` when the `flags` granularity field carries an
    ///   encoding the kernel does not accept.
    /// - `CELL_ENOMEM` when `size` does not fit in `u32`.
    /// - `CELL_EALIGN` when `size` is not a multiple of the granule
    ///   the `flags` field selects.
    /// - `CELL_EFAULT` when `mem_id_ptr` is null. RPCS3 has no such
    ///   gate -- it writes through the pointer once the shm exists --
    ///   so the gate sits after the argument refusals and before any
    ///   id or ipc-key registration.
    pub(super) fn dispatch_mmapper_allocate_shared_memory(
        &mut self,
        args: [u64; 8],
        requester: UnitId,
        tick: GuestTicks,
    ) -> Lv2Dispatch {
        let ipc_key = args[0];
        let size = args[1];
        let flags = args[2];
        let mem_id_ptr = args[3] as u32;
        if size == 0 {
            return Lv2Dispatch::immediate(cell_errors::CELL_EALIGN.into());
        }
        let Some(align) = accepted_granule(flags) else {
            return Lv2Dispatch::immediate(cell_errors::CELL_EINVAL.into());
        };
        let Ok(size_u32) = u32::try_from(size) else {
            return Lv2Dispatch::immediate(cell_errors::CELL_ENOMEM.into());
        };
        if !size_u32.is_multiple_of(align) {
            return Lv2Dispatch::immediate(cell_errors::CELL_EALIGN.into());
        }
        if let Some(d) = self.efault_if_null(&[mem_id_ptr]) {
            return d;
        }
        let keyed = ipc_key != 0 && ipc_key != SYS_MMAPPER_NO_SHM_KEY;
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
        let write = Effect::SharedWriteIntent {
            range: ByteRange::contiguous_u32(mem_id_ptr, 4),
            bytes: WritePayload::from_slice(&mem_id.to_be_bytes()),
            ordering: PriorityClass::Normal,
            source: requester,
            source_time: tick,
        };
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
                Effect::SharedWriteIntent {
                    range: ByteRange::contiguous_u32(
                        base_addr.wrapping_add(*offset),
                        bytes.len() as u32,
                    ),
                    bytes: WritePayload::from_slice(bytes),
                    ordering: PriorityClass::Normal,
                    source: requester,
                    source_time: tick,
                }
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
    ///   prior maps alike (RPCS3 sys_mmapper.cpp
    ///   sys_mmapper_map_shared_memory when the window cannot be
    ///   claimed).
    pub(super) fn dispatch_mmapper_map_shared_memory(
        &mut self,
        args: [u64; 8],
        requester: UnitId,
        rt: &dyn crate::host::Lv2Runtime,
        tick: GuestTicks,
    ) -> Lv2Dispatch {
        let addr = args[0];
        let mem_id = args[1] as u32;
        if !(0x2000_0000..0xC000_0000).contains(&addr) {
            return Lv2Dispatch::immediate(cell_errors::CELL_EINVAL.into());
        }
        let Some(handle) = self.state.mmapper_handles.get(mem_id) else {
            self.log_invariant_break(
                "dispatch.mmapper_map_unknown_mem_id",
                format_args!(
                    "sys_mmapper_map_shared_memory mem_id={mem_id:#x} not in handle table; \
                     332 / 362 must precede 334"
                ),
            );
            return Lv2Dispatch::immediate(cell_errors::CELL_ESRCH.into());
        };
        if !addr.is_multiple_of(u64::from(handle.align)) {
            return Lv2Dispatch::immediate(cell_errors::CELL_EALIGN.into());
        }
        let Some(end) = addr.checked_add(u64::from(handle.size)) else {
            return Lv2Dispatch::immediate(cell_errors::CELL_EINVAL.into());
        };
        if end > 0xC000_0000 {
            return Lv2Dispatch::immediate(cell_errors::CELL_EINVAL.into());
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
            return Lv2Dispatch::immediate(cell_errors::CELL_EBUSY.into());
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
    /// Oracle: RPCS3
    /// `tools/rpcs3-src/rpcs3/Emu/Cell/lv2/sys_mmapper.cpp`
    /// `sys_mmapper_search_and_map`.
    /// RPCS3 calls `area->alloc(...)` which searches the area; the
    /// out-pointer receives the actually mapped address, not the
    /// caller's hint.
    ///
    /// # Errors
    /// - `CELL_EFAULT` when `alloc_addr_ptr` is null.
    /// - `CELL_EINVAL` when `start_addr` is outside the mmapper
    ///   window `[MMAPPER_REGION_START, MMAPPER_REGION_END)`.
    /// - `CELL_ESRCH` when `mem_id` is not in the handle table
    ///   (parallel to sc 334's `mmapper_map_unknown_mem_id` shape;
    ///   logs `dispatch.mmapper_search_and_map_unknown_mem_id`).
    /// - `CELL_ENOMEM` when the search exhausts the mmapper window.
    pub(super) fn dispatch_mmapper_search_and_map(
        &mut self,
        args: [u64; 8],
        requester: UnitId,
        rt: &dyn crate::host::Lv2Runtime,
        tick: GuestTicks,
    ) -> Lv2Dispatch {
        let start_addr = args[0] as u32;
        let mem_id = args[1] as u32;
        let alloc_addr_ptr = args[3] as u32;
        if let Some(d) = self.efault_if_null(&[alloc_addr_ptr]) {
            return d;
        }
        if !(Lv2Host::MMAPPER_REGION_START..Lv2Host::MMAPPER_REGION_END).contains(&start_addr) {
            return Lv2Dispatch::immediate(cell_errors::CELL_EINVAL.into());
        }
        let Some(handle) = self.state.mmapper_handles.get(mem_id) else {
            self.log_invariant_break(
                "dispatch.mmapper_search_and_map_unknown_mem_id",
                format_args!(
                    "sys_mmapper_search_and_map mem_id={mem_id:#x} not in handle table; \
                     332 / 362 must precede 337"
                ),
            );
            return Lv2Dispatch::immediate(cell_errors::CELL_ESRCH.into());
        };
        let Some(found_addr) =
            self.mmapper_search_free_range(start_addr, handle.size, handle.align, rt)
        else {
            return Lv2Dispatch::immediate(cell_errors::CELL_ENOMEM.into());
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
        effects.push(Effect::SharedWriteIntent {
            range: ByteRange::contiguous_u32(alloc_addr_ptr, 4),
            bytes: WritePayload::from_slice(&found_addr.to_be_bytes()),
            ordering: PriorityClass::Normal,
            source: requester,
            source_time: tick,
        });
        Lv2Dispatch::Immediate { code: 0, effects }
    }

    /// `sys_mmapper_allocate_shared_memory_from_container` (362):
    /// container variant of 332, with flags at r6 (`args[3]`) and
    /// `mem_id` out-pointer at r7.
    ///
    /// # Errors
    ///
    /// Same set and same order as
    /// `dispatch_mmapper_allocate_shared_memory`.
    pub(super) fn dispatch_mmapper_allocate_shared_memory_from_container(
        &mut self,
        args: [u64; 8],
        requester: UnitId,
        tick: GuestTicks,
    ) -> Lv2Dispatch {
        let size = args[1];
        let flags = args[3];
        let mem_id_ptr = args[4] as u32;
        if size == 0 {
            return Lv2Dispatch::immediate(cell_errors::CELL_EALIGN.into());
        }
        let Some(align) = accepted_granule(flags) else {
            return Lv2Dispatch::immediate(cell_errors::CELL_EINVAL.into());
        };
        let Ok(size_u32) = u32::try_from(size) else {
            return Lv2Dispatch::immediate(cell_errors::CELL_ENOMEM.into());
        };
        if !size_u32.is_multiple_of(align) {
            return Lv2Dispatch::immediate(cell_errors::CELL_EALIGN.into());
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
        let write = Effect::SharedWriteIntent {
            range: ByteRange::contiguous_u32(mem_id_ptr, 4),
            bytes: WritePayload::from_slice(&mem_id.to_be_bytes()),
            ordering: PriorityClass::Normal,
            source: requester,
            source_time: tick,
        };
        Lv2Dispatch::Immediate {
            code: 0,
            effects: vec![write],
        }
    }

    /// `sys_tty_read` (402): returns EIO (matches retail LV2 outside
    /// debug-console mode).
    pub(super) fn dispatch_tty_read(&self) -> Lv2Dispatch {
        Lv2Dispatch::immediate(cell_errors::CELL_EIO.into())
    }

    /// DEX-only slot (462): retail liblv2 takes its fallback path on
    /// ENOSYS.
    pub(super) fn dispatch_uns_func_462(&self) -> Lv2Dispatch {
        Lv2Dispatch::immediate(cell_errors::CELL_ENOSYS.into())
    }

    /// `_sys_prx_start_module` (481): the two-phase start handshake.
    ///
    /// `pOpt->cmd & 0xF` selects the phase. Phase 1 hands the caller
    /// the entry to invoke; phase 2 reports what that entry returned.
    ///
    /// CellGov runs every firmware module's `module_start` itself at
    /// boot, in dependency order, so phase 1 reports `NO_ENTRY`
    /// rather than the real OPD -- handing back the real address
    /// would run `module_start` a second time. Phase 2 reporting
    /// `SYS_PRX_RESIDENT` marks the module started, which is what
    /// makes a later unload answer `NOT_REMOVABLE`.
    ///
    /// `pOpt->size` is never validated -- RPCS3's handler reads the
    /// fields unconditionally and consults `size` only to decide
    /// whether `entry2` exists (`sys_prx.cpp`).
    ///
    /// # Errors
    ///
    /// - `CELL_EINVAL` for null `id` or null `pOpt`.
    /// - `CELL_ESRCH` when `id` names no loaded module.
    /// - `CELL_PRX_ERROR_ERROR` for an unrecognised command nibble
    ///   (RPCS3's default arm).
    /// - `CELL_EFAULT` when `pOpt` is unreadable or the struct would
    ///   not fit inside the 32-bit guest address space.
    pub(super) fn dispatch_prx_start_module(
        &mut self,
        args: [u64; 8],
        requester: UnitId,
        rt: &dyn Lv2Runtime,
        tick: GuestTicks,
    ) -> Lv2Dispatch {
        use cellgov_ps3_abi::sys_prx::{
            start_cmd, start_stop_option as opt, CELL_PRX_ERROR_ERROR, SYS_PRX_RESIDENT,
        };

        let id = args[0] as u32;
        let p_opt = args[2] as u32;
        if id == 0 || p_opt == 0 {
            return Lv2Dispatch::immediate(cell_errors::CELL_EINVAL.into());
        }
        if self.state.prx_registry.lookup_by_id(id).is_none() {
            return Lv2Dispatch::immediate(cell_errors::CELL_ESRCH.into());
        }
        // The base struct (through `res`) must fit below the 4 GiB
        // boundary; `entry2`'s reach is gated after `size` is known.
        if p_opt.checked_add(opt::MIN_SIZE as u32).is_none() {
            self.log_invariant_break(
                "dispatch.prx_start_module_p_opt_wraps",
                format_args!(
                    "_sys_prx_start_module option struct at p_opt={p_opt:#010x} wraps u32; \
                     returning CELL_EFAULT (struct does not fit in 32-bit guest address space)"
                ),
            );
            return Lv2Dispatch::immediate(cell_errors::CELL_EFAULT.into());
        }

        let Some(size) = self.read_be_u64(rt, p_opt + opt::SIZE_OFFSET) else {
            self.log_invariant_break(
                "dispatch.prx_start_module_size_unreadable",
                format_args!(
                    "_sys_prx_start_module pOpt={p_opt:#010x} size field unreadable; \
                     returning CELL_EFAULT"
                ),
            );
            return Lv2Dispatch::immediate(cell_errors::CELL_EFAULT.into());
        };
        // Extended struct: entry2 at +0x20 must also fit.
        if size != opt::MIN_SIZE && p_opt.checked_add(opt::ENTRY2_OFFSET + 8).is_none() {
            self.log_invariant_break(
                "dispatch.prx_start_module_entry2_wraps",
                format_args!(
                    "_sys_prx_start_module extended option struct at p_opt={p_opt:#010x} \
                     (size={size:#x}) wraps u32 at entry2; returning CELL_EFAULT"
                ),
            );
            return Lv2Dispatch::immediate(cell_errors::CELL_EFAULT.into());
        }
        let Some(cmd) = self.read_be_u64(rt, p_opt + opt::CMD_OFFSET) else {
            self.log_invariant_break(
                "dispatch.prx_start_module_cmd_unreadable",
                format_args!(
                    "_sys_prx_start_module pOpt={p_opt:#010x} cmd field unreadable; \
                     returning CELL_EFAULT"
                ),
            );
            return Lv2Dispatch::immediate(cell_errors::CELL_EFAULT.into());
        };

        match cmd & start_cmd::MASK {
            start_cmd::GET_ENTRY => {
                let mut effects = vec![self.write_be_u64(
                    requester,
                    p_opt + opt::ENTRY_OFFSET,
                    opt::NO_ENTRY,
                    tick,
                )];
                // entry2 exists only in the extended struct.
                if size != opt::MIN_SIZE {
                    effects.push(self.write_be_u64(
                        requester,
                        p_opt + opt::ENTRY2_OFFSET,
                        opt::NO_ENTRY,
                        tick,
                    ));
                }
                Lv2Dispatch::Immediate { code: 0, effects }
            }
            start_cmd::REPORT_RESULT => {
                let Some(res) = self.read_be_u64(rt, p_opt + opt::RES_OFFSET) else {
                    self.log_invariant_break(
                        "dispatch.prx_start_module_res_unreadable",
                        format_args!(
                            "_sys_prx_start_module pOpt={p_opt:#010x} res field unreadable; \
                             returning CELL_EFAULT"
                        ),
                    );
                    return Lv2Dispatch::immediate(cell_errors::CELL_EFAULT.into());
                };
                if res == SYS_PRX_RESIDENT {
                    // LV2's STARTING -> STARTED transition: the module
                    // is now resident and refuses unload.
                    self.state.prx_registry.mark_started(id);
                    return Lv2Dispatch::immediate(0);
                }
                // Phase 1 handed back NO_ENTRY, so the guest called
                // nothing and cannot have a real result to report.
                // Mirroring RPCS3, the low 32 bits come back as the
                // error; a zero low word therefore returns CELL_OK.
                self.log_invariant_break(
                    "dispatch.prx_start_module_unexpected_res",
                    format_args!(
                        "_sys_prx_start_module id={id:#010x} cmd=2 reported res={res:#018x} \
                         after phase 1 returned NO_ENTRY; returning res & 0xFFFF_FFFF \
                         (CELL_OK when the low word is zero)"
                    ),
                );
                Lv2Dispatch::immediate(res & 0xFFFF_FFFF)
            }
            other => {
                self.log_invariant_break(
                    "dispatch.prx_start_module_unknown_cmd",
                    format_args!(
                        "_sys_prx_start_module id={id:#010x} cmd nibble {other:#x} is not \
                         GET_ENTRY(1) or REPORT_RESULT(2); returning CELL_PRX_ERROR_ERROR \
                         (RPCS3's default arm)"
                    ),
                );
                Lv2Dispatch::immediate(CELL_PRX_ERROR_ERROR.into())
            }
        }
    }

    /// `_sys_prx_stop_module` (482): the two-phase stop handshake.
    ///
    /// `pOpt->cmd & 0xF` selects the arm, mirroring sc 481. Phase 1
    /// moves a `Started` module to `Stopping` and hands the caller
    /// the entry to invoke; phase 2 reporting `res == 0` completes
    /// `Stopping -> Stopped`, after which unload withdraws the
    /// module.
    ///
    /// CellGov never runs guest `module_stop` (modules were started
    /// host-side at boot without running guest code), so phase 1
    /// reports `NO_ENTRY` exactly as sc 481 does; liblv2 then skips
    /// the call and reports zero.
    ///
    /// Unlike sc 481, the id lookup precedes the null-`pOpt` gate:
    /// RPCS3's 482 orders ESRCH before EINVAL (`sys_prx.cpp`), and a
    /// zero id is just an id the lookup misses.
    ///
    /// # Errors
    ///
    /// - `CELL_ESRCH` when `id` names no loaded module.
    /// - `CELL_EINVAL` for null `pOpt`.
    /// - `CELL_PRX_ERROR_NOT_STARTED` / `ALREADY_STOPPED` /
    ///   `ALREADY_STOPPING` when cmd 1 / 4 / 8 finds the module in
    ///   the wrong state.
    /// - `CELL_PRX_ERROR_CAN_NOT_STOP` when phase 2 reports `res == 1`.
    /// - `CELL_PRX_ERROR_ERROR` for an unrecognised command nibble
    ///   (RPCS3's default arm).
    /// - `CELL_EFAULT` when `pOpt` is unreadable or the struct would
    ///   not fit inside the 32-bit guest address space.
    pub(super) fn dispatch_prx_stop_module(
        &mut self,
        args: [u64; 8],
        requester: UnitId,
        rt: &dyn Lv2Runtime,
        tick: GuestTicks,
    ) -> Lv2Dispatch {
        use crate::prx_registry::PrxState;
        use cellgov_ps3_abi::sys_prx::{
            start_cmd, start_stop_option as opt, stop_cmd, CELL_PRX_ERROR_ALREADY_STOPPED,
            CELL_PRX_ERROR_ALREADY_STOPPING, CELL_PRX_ERROR_CAN_NOT_STOP, CELL_PRX_ERROR_ERROR,
            CELL_PRX_ERROR_NOT_STARTED,
        };

        let id = args[0] as u32;
        let p_opt = args[2] as u32;
        if self.state.prx_registry.lookup_by_id(id).is_none() {
            return Lv2Dispatch::immediate(cell_errors::CELL_ESRCH.into());
        }
        if p_opt == 0 {
            return Lv2Dispatch::immediate(cell_errors::CELL_EINVAL.into());
        }
        // The base struct (through `res`) must fit below the 4 GiB
        // boundary; `entry2`'s reach is gated after `size` is known.
        if p_opt.checked_add(opt::MIN_SIZE as u32).is_none() {
            self.log_invariant_break(
                "dispatch.prx_stop_module_p_opt_wraps",
                format_args!(
                    "_sys_prx_stop_module option struct at p_opt={p_opt:#010x} wraps u32; \
                     returning CELL_EFAULT (struct does not fit in 32-bit guest address space)"
                ),
            );
            return Lv2Dispatch::immediate(cell_errors::CELL_EFAULT.into());
        }

        let Some(size) = self.read_be_u64(rt, p_opt + opt::SIZE_OFFSET) else {
            self.log_invariant_break(
                "dispatch.prx_stop_module_size_unreadable",
                format_args!(
                    "_sys_prx_stop_module pOpt={p_opt:#010x} size field unreadable; \
                     returning CELL_EFAULT"
                ),
            );
            return Lv2Dispatch::immediate(cell_errors::CELL_EFAULT.into());
        };
        // Extended struct: entry2 at +0x20 must also fit.
        if size != opt::MIN_SIZE && p_opt.checked_add(opt::ENTRY2_OFFSET + 8).is_none() {
            self.log_invariant_break(
                "dispatch.prx_stop_module_entry2_wraps",
                format_args!(
                    "_sys_prx_stop_module extended option struct at p_opt={p_opt:#010x} \
                     (size={size:#x}) wraps u32 at entry2; returning CELL_EFAULT"
                ),
            );
            return Lv2Dispatch::immediate(cell_errors::CELL_EFAULT.into());
        }
        let Some(cmd) = self.read_be_u64(rt, p_opt + opt::CMD_OFFSET) else {
            self.log_invariant_break(
                "dispatch.prx_stop_module_cmd_unreadable",
                format_args!(
                    "_sys_prx_stop_module pOpt={p_opt:#010x} cmd field unreadable; \
                     returning CELL_EFAULT"
                ),
            );
            return Lv2Dispatch::immediate(cell_errors::CELL_EFAULT.into());
        };

        // Write NO_ENTRY to `entry` (and `entry2` for the extended
        // struct); shared by the cmd 1 and cmd 4 arms.
        let no_entry_effects = |host: &Self| {
            let mut effects =
                vec![host.write_be_u64(requester, p_opt + opt::ENTRY_OFFSET, opt::NO_ENTRY, tick)];
            if size != opt::MIN_SIZE {
                effects.push(host.write_be_u64(
                    requester,
                    p_opt + opt::ENTRY2_OFFSET,
                    opt::NO_ENTRY,
                    tick,
                ));
            }
            effects
        };
        let wrong_state = |state: PrxState| match state {
            PrxState::Initialized => Some(CELL_PRX_ERROR_NOT_STARTED),
            PrxState::Stopped => Some(CELL_PRX_ERROR_ALREADY_STOPPED),
            PrxState::Stopping => Some(CELL_PRX_ERROR_ALREADY_STOPPING),
            PrxState::Started => None,
        };

        match cmd & start_cmd::MASK {
            start_cmd::GET_ENTRY => {
                let old = self
                    .state
                    .prx_registry
                    .begin_stop(id)
                    .expect("id lookup succeeded above");
                if let Some(err) = wrong_state(old) {
                    return Lv2Dispatch::immediate(err.into());
                }
                Lv2Dispatch::Immediate {
                    code: 0,
                    effects: no_entry_effects(self),
                }
            }
            start_cmd::REPORT_RESULT => {
                let Some(res) = self.read_be_u64(rt, p_opt + opt::RES_OFFSET) else {
                    self.log_invariant_break(
                        "dispatch.prx_stop_module_res_unreadable",
                        format_args!(
                            "_sys_prx_stop_module pOpt={p_opt:#010x} res field unreadable; \
                             returning CELL_EFAULT"
                        ),
                    );
                    return Lv2Dispatch::immediate(cell_errors::CELL_EFAULT.into());
                };
                match res {
                    0 => {
                        if !self.state.prx_registry.finish_stop(id) {
                            // RPCS3 hard-asserts STOPPING here; a
                            // phase 2 with no accepted phase 1 has no
                            // oracle behaviour to mirror.
                            self.log_invariant_break(
                                "dispatch.prx_stop_module_unexpected_state",
                                format_args!(
                                    "_sys_prx_stop_module id={id:#010x} cmd=2 res=0 but the \
                                     module is not in the Stopping state; returning CELL_OK \
                                     without a transition"
                                ),
                            );
                        }
                        Lv2Dispatch::immediate(0)
                    }
                    1 => {
                        // Phase 1 handed back NO_ENTRY, so the guest
                        // called nothing; a can-not-stop report is a
                        // gap signal even though the code is faithful.
                        self.log_invariant_break(
                            "dispatch.prx_stop_module_unexpected_res",
                            format_args!(
                                "_sys_prx_stop_module id={id:#010x} cmd=2 reported res=1 after \
                                 phase 1 returned NO_ENTRY; returning \
                                 CELL_PRX_ERROR_CAN_NOT_STOP"
                            ),
                        );
                        Lv2Dispatch::immediate(CELL_PRX_ERROR_CAN_NOT_STOP.into())
                    }
                    other => {
                        // RPCS3: "Nothing happens (probably
                        // unexpected value)".
                        self.log_invariant_break(
                            "dispatch.prx_stop_module_unexpected_res",
                            format_args!(
                                "_sys_prx_stop_module id={id:#010x} cmd=2 reported \
                                 res={other:#018x} after phase 1 returned NO_ENTRY; returning \
                                 CELL_OK with no state change (RPCS3's default arm)"
                            ),
                        );
                        Lv2Dispatch::immediate(0)
                    }
                }
            }
            stop_cmd::GET_ENTRIES | stop_cmd::DISABLE_STOP => {
                let state = self
                    .state
                    .prx_registry
                    .lookup_by_id(id)
                    .expect("id lookup succeeded above")
                    .state();
                if let Some(err) = wrong_state(state) {
                    return Lv2Dispatch::immediate(err.into());
                }
                // RPCS3 selects the arm by nibble but branches on the
                // FULL cmd value: only exactly 4 reads the entries;
                // 8, 0x14, 0x18, ... all take the disable-stop path.
                if cmd == stop_cmd::GET_ENTRIES {
                    return Lv2Dispatch::Immediate {
                        code: 0,
                        effects: no_entry_effects(self),
                    };
                }
                self.log_invariant_break(
                    "dispatch.prx_stop_module_disable_stop_stub",
                    format_args!(
                        "_sys_prx_stop_module id={id:#010x} cmd={cmd:#x} disable-stop is a \
                         no-op stub returning CELL_OK; matches RPCS3's todo arm"
                    ),
                );
                Lv2Dispatch::immediate(0)
            }
            other => {
                self.log_invariant_break(
                    "dispatch.prx_stop_module_unknown_cmd",
                    format_args!(
                        "_sys_prx_stop_module id={id:#010x} cmd nibble {other:#x} is not 1, 2, \
                         4, or 8; returning CELL_PRX_ERROR_ERROR (RPCS3's default arm)"
                    ),
                );
                Lv2Dispatch::immediate(CELL_PRX_ERROR_ERROR.into())
            }
        }
    }

    /// `_sys_prx_unload_module` (483): withdraw an `Initialized` or
    /// `Stopped` module, refuse a `Started` / `Stopping` one.
    ///
    /// LV2 (and RPCS3's `_sys_prx_unload_module`) withdraws a module
    /// in `INITIALIZED` or `STOPPED` state and answers
    /// `CELL_PRX_ERROR_NOT_REMOVABLE` otherwise. Boot-loaded firmware
    /// modules are started (their images back the GOT slots the title
    /// calls through), so they refuse until the guest completes the
    /// sc 482 stop handshake; an sc 480 miss stub the guest never
    /// started withdraws with `CELL_OK` and frees its id, exactly as
    /// a real never-started module would.
    ///
    /// # Errors
    ///
    /// - `CELL_PRX_ERROR_UNKNOWN_MODULE` when `id` names no loaded module.
    /// - `CELL_PRX_ERROR_NOT_REMOVABLE` for a started or stopping
    ///   resident module.
    pub(super) fn dispatch_prx_unload_module(&mut self, args: [u64; 8]) -> Lv2Dispatch {
        use crate::prx_registry::PrxState;
        use cellgov_ps3_abi::sys_prx::{
            CELL_PRX_ERROR_NOT_REMOVABLE, CELL_PRX_ERROR_UNKNOWN_MODULE,
        };

        let id = args[0] as u32;
        match self.state.prx_registry.lookup_by_id(id) {
            None => Lv2Dispatch::immediate(CELL_PRX_ERROR_UNKNOWN_MODULE.into()),
            Some(entry) => match entry.state() {
                PrxState::Initialized | PrxState::Stopped => {
                    let removed = self.state.prx_registry.withdraw_removable(id);
                    debug_assert!(removed.is_some(), "lookup said present and removable");
                    Lv2Dispatch::immediate(0)
                }
                PrxState::Started | PrxState::Stopping => {
                    let stem = entry.stem().to_string();
                    self.obs.prx_unload_rejections += 1;
                    self.log_invariant_break(
                        "dispatch.prx_unload_module_resident",
                        format_args!(
                            "_sys_prx_unload_module id={id:#010x} ({stem}) is a started \
                             resident module; returning CELL_PRX_ERROR_NOT_REMOVABLE"
                        ),
                    );
                    Lv2Dispatch::immediate(CELL_PRX_ERROR_NOT_REMOVABLE.into())
                }
            },
        }
    }

    /// Read a big-endian `u64` from committed guest memory.
    fn read_be_u64(&self, rt: &dyn Lv2Runtime, addr: u32) -> Option<u64> {
        let bytes = rt.read_committed(u64::from(addr), 8)?;
        Some(u64::from_be_bytes(bytes[..8].try_into().ok()?))
    }

    /// Stage a big-endian `u64` write to guest memory.
    fn write_be_u64(&self, requester: UnitId, addr: u32, value: u64, tick: GuestTicks) -> Effect {
        Effect::SharedWriteIntent {
            range: ByteRange::contiguous_u32(addr, 8),
            bytes: WritePayload::from_slice(&value.to_be_bytes()),
            ordering: PriorityClass::Normal,
            source: requester,
            source_time: tick,
        }
    }

    /// `_sys_prx_register_module` (484): returns
    /// CELL_PRX_ERROR_ELF_IS_REGISTERED for non-VSH callers.
    pub(super) fn dispatch_prx_register_module(
        &mut self,
        args: [u64; 8],
        requester: UnitId,
        rt: &dyn Lv2Runtime,
        tick: GuestTicks,
    ) -> Lv2Dispatch {
        use cellgov_ps3_abi::sys_prx::CELL_PRX_ERROR_ELF_IS_REGISTERED;

        let opt = args[1];
        if opt == 0 {
            return Lv2Dispatch::immediate(cell_errors::CELL_EINVAL.into());
        }
        let Some(size) = read_be_u64(rt, opt) else {
            return Lv2Dispatch::immediate(cell_errors::CELL_EFAULT.into());
        };
        // 0x1c / 0x20 are the legacy option forms. The oracle rebuilds
        // them with type = 0, which skips the branch entirely, so they
        // need no field reads here.
        let (module_type, stub_ea, stub_size) = match size {
            0x1c | 0x20 => (0u64, 0u32, 0u32),
            0x30 => {
                let Some(t) = read_be_u64(rt, opt + 0x08) else {
                    return Lv2Dispatch::immediate(cell_errors::CELL_EFAULT.into());
                };
                let (Some(ea), Some(sz)) =
                    (read_be_u32(rt, opt + 0x20), read_be_u32(rt, opt + 0x24))
                else {
                    return Lv2Dispatch::immediate(cell_errors::CELL_EFAULT.into());
                };
                (t, ea, sz)
            }
            _ => return Lv2Dispatch::immediate(cell_errors::CELL_EINVAL.into()),
        };
        self.obs.prx_register_module_count += 1;

        if module_type & 0x1 == 0 {
            return Lv2Dispatch::immediate(0);
        }
        if !self.is_coreos() {
            // Only a CoreOS process may hand the kernel its own tables.
            return Lv2Dispatch::immediate(CELL_PRX_ERROR_ELF_IS_REGISTERED.into());
        }
        self.obs.prx_register_module_manual_count += 1;
        let effects = self.link_manual_imports(stub_ea, stub_size, requester, rt, tick);
        Lv2Dispatch::Immediate { code: 0, effects }
    }

    /// Bind the import table at `[stub_ea, stub_ea + stub_size)` against
    /// the firmware export map, returning one `SharedWriteIntent` per
    /// resolved GOT slot.
    ///
    /// The caller-supplied table is guest data and may be uninitialised
    /// -- a CoreOS module can reach this arm with a garbage pointer and
    /// a huge size. Every read is bounds-checked through the runtime and
    /// a failed read ends the walk rather than faulting the host; an
    /// unresolved NID is left alone so the guest's own stub address
    /// stays in the slot. Each way the walk can stop early names its
    /// own invariant break.
    fn link_manual_imports(
        &mut self,
        stub_ea: u32,
        stub_size: u32,
        requester: UnitId,
        rt: &dyn Lv2Runtime,
        tick: GuestTicks,
    ) -> Vec<Effect> {
        use cellgov_ps3_abi::elf::{
            PRX_IMPORT_ENTRY_MIN_SIZE, PRX_IMPORT_NAME_PTR_OFFSET, PRX_IMPORT_NIDS_PTR_OFFSET,
            PRX_IMPORT_NUM_FUNC_OFFSET, PRX_IMPORT_SIZE_OFFSET, PRX_IMPORT_STUB_PTR_OFFSET,
            PRX_NAME_MAX_LEN,
        };

        let mut effects = Vec::new();
        let Some(table_end) = stub_ea.checked_add(stub_size) else {
            self.log_invariant_break(
                "dispatch.prx_register_module_table_wraps",
                format_args!(
                    "import table [0x{stub_ea:08x}, +0x{stub_size:x}) wraps u32; \
                     nothing linked"
                ),
            );
            return effects;
        };
        let mut cursor = stub_ea;
        while cursor < table_end {
            let Some(hdr) =
                rt.read_committed(u64::from(cursor), PRX_IMPORT_ENTRY_MIN_SIZE as usize)
            else {
                self.log_invariant_break(
                    "dispatch.prx_register_module_entry_unreadable",
                    format_args!(
                        "import entry header at 0x{cursor:08x} is unreadable; the walk \
                         stops short of the table end 0x{table_end:08x}"
                    ),
                );
                break;
            };
            let entry_size = hdr[PRX_IMPORT_SIZE_OFFSET];
            if entry_size < PRX_IMPORT_ENTRY_MIN_SIZE {
                self.log_invariant_break(
                    "dispatch.prx_register_module_entry_size_invalid",
                    format_args!(
                        "import entry at 0x{cursor:08x} declares size {entry_size} below the \
                         {PRX_IMPORT_ENTRY_MIN_SIZE}-byte minimum; the walk stops short of \
                         the table end 0x{table_end:08x}"
                    ),
                );
                break;
            }
            let func_count = u16::from_be_bytes([
                hdr[PRX_IMPORT_NUM_FUNC_OFFSET],
                hdr[PRX_IMPORT_NUM_FUNC_OFFSET + 1],
            ]);
            let nids_ptr = u32::from_be_bytes([
                hdr[PRX_IMPORT_NIDS_PTR_OFFSET],
                hdr[PRX_IMPORT_NIDS_PTR_OFFSET + 1],
                hdr[PRX_IMPORT_NIDS_PTR_OFFSET + 2],
                hdr[PRX_IMPORT_NIDS_PTR_OFFSET + 3],
            ]);
            let stub_ptr = u32::from_be_bytes([
                hdr[PRX_IMPORT_STUB_PTR_OFFSET],
                hdr[PRX_IMPORT_STUB_PTR_OFFSET + 1],
                hdr[PRX_IMPORT_STUB_PTR_OFFSET + 2],
                hdr[PRX_IMPORT_STUB_PTR_OFFSET + 3],
            ]);
            let name_ptr = u32::from_be_bytes([
                hdr[PRX_IMPORT_NAME_PTR_OFFSET],
                hdr[PRX_IMPORT_NAME_PTR_OFFSET + 1],
                hdr[PRX_IMPORT_NAME_PTR_OFFSET + 2],
                hdr[PRX_IMPORT_NAME_PTR_OFFSET + 3],
            ]);
            // The cap and the lossy decode mirror the loader-side
            // decoders that mint the `firmware_exports` keys
            // (`cellgov_ppu::prx::read_cstring`,
            // `cellgov_ppu::sprx::parse::read_cstring`): a name the
            // loader accepted must produce the identical lookup key
            // here, or the library silently never resolves.
            let library = match rt.read_committed_until(u64::from(name_ptr), PRX_NAME_MAX_LEN, 0) {
                Some(bytes) => self
                    .derived
                    .firmware_exports
                    .get(String::from_utf8_lossy(bytes).as_ref()),
                None => {
                    // An unreadable name resolves under no library:
                    // the entry's NIDs still walk (so the witness
                    // counts them) but every lookup misses.
                    self.log_invariant_break(
                        "dispatch.prx_register_module_name_unreadable",
                        format_args!(
                            "import entry at 0x{cursor:08x}: library-name pointer \
                             0x{name_ptr:08x} is unreadable or unterminated within \
                             {PRX_NAME_MAX_LEN} bytes; its {func_count} import NID(s) \
                             stay unresolved",
                        ),
                    );
                    None
                }
            };
            for i in 0..u32::from(func_count) {
                let (Some(nid_at), Some(slot_at)) = (
                    nids_ptr.checked_add(i * 4).map(u64::from),
                    stub_ptr.checked_add(i * 4).map(u64::from),
                ) else {
                    self.log_invariant_break(
                        "dispatch.prx_register_module_nid_table_truncated",
                        format_args!(
                            "import entry at 0x{cursor:08x}: NID slot {i} of {func_count} \
                             wraps u32 (nids=0x{nids_ptr:08x} stubs=0x{stub_ptr:08x}); the \
                             remaining NIDs stay unresolved"
                        ),
                    );
                    break;
                };
                let Some(nid) = read_be_u32(rt, nid_at) else {
                    self.log_invariant_break(
                        "dispatch.prx_register_module_nid_table_truncated",
                        format_args!(
                            "import entry at 0x{cursor:08x}: NID {i} of {func_count} at \
                             0x{nid_at:08x} is unreadable; the remaining NIDs stay unresolved"
                        ),
                    );
                    break;
                };
                let Some(&opd) = library.and_then(|lib| lib.get(&nid)) else {
                    self.obs.prx_register_module_unresolved += 1;
                    continue;
                };
                effects.push(Effect::SharedWriteIntent {
                    range: ByteRange::contiguous_u32(slot_at as u32, 4),
                    bytes: WritePayload::from_slice(&opd.to_be_bytes()),
                    ordering: PriorityClass::Normal,
                    source: requester,
                    source_time: tick,
                });
                self.obs.prx_register_module_linked += 1;
            }
            let Some(next) = cursor.checked_add(u32::from(entry_size)) else {
                self.log_invariant_break(
                    "dispatch.prx_register_module_entry_advance_wraps",
                    format_args!(
                        "import entry at 0x{cursor:08x} plus its size {entry_size} wraps u32; \
                         the walk stops short of the table end 0x{table_end:08x}"
                    ),
                );
                break;
            };
            cursor = next;
        }
        effects
    }

    /// `_sys_prx_register_library` (486): gates the library
    /// descriptor, then returns CELL_OK (the kernel's no-match
    /// success path).
    ///
    /// Associating the descriptor with a loaded module's export table
    /// is not modelled; CellGov publishes every firmware module's
    /// exports at boot, so a caller-registered library adds no
    /// resolvable symbol.
    ///
    /// # Errors
    ///
    /// - `CELL_EFAULT` when `library` is null or unmapped. RPCS3's
    ///   `sys_prx.cpp` `_sys_prx_register_library` refuses an address
    ///   that fails its mapping check before touching the descriptor.
    pub(super) fn dispatch_prx_register_library(
        &self,
        args: [u64; 8],
        rt: &dyn Lv2Runtime,
    ) -> Lv2Dispatch {
        let library = args[0];
        if library == 0 || rt.read_committed(library, 1).is_none() {
            return Lv2Dispatch::immediate(cell_errors::CELL_EFAULT.into());
        }
        Lv2Dispatch::immediate(0)
    }

    /// `_sys_prx_get_module_list` (494): fills `pInfo->idlist` and
    /// writes `pInfo->count`, filtering liblv2.sprx.
    ///
    /// Struct layout (RPCS3 `sys_prx.h`
    /// `sys_prx_get_module_list_option_t`): `size@0` (u64), `pad@8`,
    /// `max@0xC`, `count@0x10`, `idlist@0x14`, `unk@0x18`, tail
    /// padding to 0x20. Only `[p_info, p_info+0x18)` is touched.
    /// `flags & 0x2 == 0` short-circuits to CELL_OK. CELL_EFAULT on
    /// null `pInfo`.
    ///
    /// # Cross-module contract
    ///
    /// Slot writes and the trailing count write are co-emitted in one
    /// `Lv2Dispatch::Immediate` batch so `apply_lv2_effects` can
    /// commit them all-or-none.
    pub(super) fn dispatch_prx_get_module_list(
        &mut self,
        args: [u64; 8],
        requester: UnitId,
        rt: &dyn Lv2Runtime,
        tick: GuestTicks,
    ) -> Lv2Dispatch {
        let flags = args[0];
        let p_info = args[1] as u32;
        if flags & 0x2 == 0 {
            return Lv2Dispatch::immediate(0);
        }
        if p_info == 0 {
            return Lv2Dispatch::immediate(cell_errors::CELL_EFAULT.into());
        }
        if p_info.checked_add(0x18).is_none() {
            self.log_invariant_break(
                "dispatch.prx_module_list_p_info_wraps",
                format_args!(
                    "sys_prx_get_module_list pInfo struct [p_info, p_info+0x18) wraps u32: \
                     pInfo={p_info:#010x}; returning CELL_EFAULT (struct does not fit in \
                     32-bit guest address space)"
                ),
            );
            return Lv2Dispatch::immediate(cell_errors::CELL_EFAULT.into());
        }
        let mut effects = Vec::new();
        let max_addr = p_info.wrapping_add(0x0C);
        let count_addr = p_info.wrapping_add(0x10);
        let idlist_ptr_addr = p_info.wrapping_add(0x14);
        let Some(max_bytes) = rt.read_committed(u64::from(max_addr), 4) else {
            self.log_invariant_break(
                "dispatch.prx_module_list_unreadable_pinfo",
                format_args!(
                    "sys_prx_get_module_list pInfo={p_info:#010x} max field at \
                     {max_addr:#010x} unreadable; returning CELL_EFAULT"
                ),
            );
            return Lv2Dispatch::immediate(cell_errors::CELL_EFAULT.into());
        };
        let max = u32::from_be_bytes([max_bytes[0], max_bytes[1], max_bytes[2], max_bytes[3]]);
        let Some(idlist_bytes) = rt.read_committed(u64::from(idlist_ptr_addr), 4) else {
            self.log_invariant_break(
                "dispatch.prx_module_list_unreadable_pinfo",
                format_args!(
                    "sys_prx_get_module_list pInfo={p_info:#010x} idlist field at \
                     {idlist_ptr_addr:#010x} unreadable; returning CELL_EFAULT"
                ),
            );
            return Lv2Dispatch::immediate(cell_errors::CELL_EFAULT.into());
        };
        let idlist_ptr = u32::from_be_bytes([
            idlist_bytes[0],
            idlist_bytes[1],
            idlist_bytes[2],
            idlist_bytes[3],
        ]);
        let liblv2_id = self
            .state
            .prx_registry
            .lookup_by_path("liblv2.sprx")
            .map(|e| e.kernel_id());
        let mut count: u32 = 0;
        if idlist_ptr != 0 {
            for kid in self.state.prx_registry.ids() {
                if Some(kid) == liblv2_id {
                    continue;
                }
                if count >= max {
                    break;
                }
                debug_assert!(
                    count
                        .checked_mul(4)
                        .and_then(|off| idlist_ptr.checked_add(off))
                        .and_then(|s| s.checked_add(4))
                        .is_some(),
                    "sys_prx_get_module_list 4-byte slot write at idlist_ptr+count*4 \
                     wraps u32: idlist_ptr={idlist_ptr:#010x} count={count}",
                );
                let slot = idlist_ptr.wrapping_add(count.wrapping_mul(4));
                effects.push(Effect::SharedWriteIntent {
                    range: ByteRange::contiguous_u32(slot, 4),
                    bytes: WritePayload::from_slice(&kid.to_be_bytes()),
                    ordering: PriorityClass::Normal,
                    source: requester,
                    source_time: tick,
                });
                count += 1;
            }
        }
        effects.push(Effect::SharedWriteIntent {
            range: ByteRange::contiguous_u32(count_addr, 4),
            bytes: WritePayload::from_slice(&count.to_be_bytes()),
            ordering: PriorityClass::Normal,
            source: requester,
            source_time: tick,
        });
        Lv2Dispatch::Immediate { code: 0, effects }
    }

    /// `sys_hid_manager_is_process_permission_root` (512): returns 0
    /// (retail titles run unprivileged).
    pub(super) fn dispatch_hid_is_root(&self) -> Lv2Dispatch {
        Lv2Dispatch::immediate(0)
    }

    /// `sys_gamepad_ycon_if` (621): stub returning CELL_OK.
    pub(super) fn dispatch_gamepad_ycon_if(&mut self) -> Lv2Dispatch {
        self.log_invariant_break(
            "dispatch.gamepad_ycon_if_stub",
            format_args!(
                "sys_gamepad_ycon_if: stub returning CELL_OK; matches RPCS3's \
                 todo-and-OK stub"
            ),
        );
        Lv2Dispatch::immediate(0)
    }

    /// `sys_rsx_attribute` (677): returns CELL_OK without state change.
    pub(super) fn dispatch_rsx_attribute(&mut self) -> Lv2Dispatch {
        self.log_invariant_break(
            "dispatch.rsx_attribute_stub",
            format_args!("sys_rsx_attribute: stub returning CELL_OK with no state change"),
        );
        Lv2Dispatch::immediate(0)
    }
}

/// The page granule a `sys_mmapper` `flags` word selects, `None` when
/// its granularity field holds an encoding the kernel refuses.
///
/// The field is `flags` bits [8,11]; unset, 64 KiB, and 1 MiB are the
/// only accepted values, and anything else is an error rather than a
/// value rounded to a granule. RPCS3's `sys_mmapper.cpp`
/// `sys_mmapper_allocate_shared_memory` switches on the field and
/// answers CELL_EINVAL from its default arm.
fn accepted_granule(flags: u64) -> Option<u32> {
    match flags & page_size::GRANULARITY_FIELD {
        0 | page_size::FLAG_64K | page_size::FLAG_1M => Some(page_size::granule_from_flags(flags)),
        _ => None,
    }
}

/// Big-endian u32 at `addr`, or `None` when the range is unmapped.
fn read_be_u32(rt: &dyn Lv2Runtime, addr: u64) -> Option<u32> {
    let b = rt.read_committed(addr, 4)?;
    Some(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
}

/// Big-endian u64 at `addr`, or `None` when the range is unmapped.
fn read_be_u64(rt: &dyn Lv2Runtime, addr: u64) -> Option<u64> {
    let b = rt.read_committed(addr, 8)?;
    Some(u64::from_be_bytes([
        b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7],
    ]))
}
