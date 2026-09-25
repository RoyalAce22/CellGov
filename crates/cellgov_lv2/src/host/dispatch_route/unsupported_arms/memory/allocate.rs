//! The `sys_mmapper` address and shared-memory allocators, and `sys_memory_container_create`.

use cellgov_effects::{Effect, WritePayload};
use cellgov_event::UnitId;
use cellgov_mem::ByteRange;
use cellgov_ps3_abi::lv2::errno::{self, Lv2ErrCode};
use cellgov_ps3_abi::lv2::memory::{
    ext_entry, page_size, CONTAINER_GRANULE, SYS_MMAPPER_NO_SHM_KEY, VM_AREA_ALIGNMENTS,
    VM_AREA_GRANULE,
};
use cellgov_time::GuestTicks;

use crate::dispatch::Lv2Dispatch;
use crate::host::guest_struct::read_be_u64;
use crate::host::mmapper::MmapperHandle;
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
        self.state.memory_containers.insert(cid, ());
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
    /// The cursor runs over `[MMAPPER_REGION_START, MMAPPER_REGION_END)`.
    /// The start sits 256 MiB above `SYS_RSX_MEM_END`, so the reserved
    /// rsx_context window below it cannot alias a handout. The end is
    /// the RSX dma_control MMIO base itself; the exclusive bound keeps
    /// it out of every handout.
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
    /// - `CELL_ENOMEM` when `size` is zero or the VM window is
    ///   exhausted.
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
        let handle = match shared_memory_handle(size, flags) {
            Ok(handle) => handle,
            Err(code) => return Lv2Dispatch::immediate(code.into()),
        };
        if let Some(d) = self.efault_if_null(&[mem_id_ptr]) {
            return d;
        }
        let keyed = names_shared_segment(ipc_key);
        let in_namespace = keyed && crate::host::is_system_ipc_key(ipc_key);
        let mem_id = match self.state.mmapper_ipc.get(ipc_key) {
            Some(&existing) if keyed => {
                if in_namespace {
                    self.obs.system_ipc_witness.shm_attaches += 1;
                    self.obs.system_ipc_witness.note_key(ipc_key);
                }
                existing
            }
            _ => {
                let mem_id = self.mint_shared_memory(handle);
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
        mem_id_written(mem_id_ptr, mem_id, requester, tick)
    }

    /// `sys_mmapper_allocate_shared_memory_from_container` (362):
    /// container variant of 332, with flags at r6 (`args[3]`) and
    /// `mem_id` out-pointer at r7.
    ///
    /// There is no ipc-key path: every call mints a fresh `mem_id`, so
    /// 332's process-shared attach does not apply. The arm does not
    /// read the container id.
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
        let handle = match shared_memory_handle(size, flags) {
            Ok(handle) => handle,
            Err(code) => return Lv2Dispatch::immediate(code.into()),
        };
        if let Some(d) = self.efault_if_null(&[mem_id_ptr]) {
            return d;
        }
        let mem_id = self.mint_shared_memory(handle);
        mem_id_written(mem_id_ptr, mem_id, requester, tick)
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
    /// - `CELL_EINVAL` when `entry_count` is outside
    ///   `1..=ext_entry::MAX_COUNT`.
    /// - `CELL_EFAULT` when an entry's `type` word is unreadable.
    /// - `CELL_EPERM` when an entry type is unknown, or privileged
    ///   without 64 KiB pages and debug-or-root capability.
    /// - `CELL_EFAULT` when `mem_id_ptr` is null.
    /// - `CELL_EEXIST` when a keyed `ipc_key` is already registered.
    ///
    /// Two gates precede the list above:
    ///
    /// - `CELL_EINVAL` when a `flags`, `entries` or `mem_id` register
    ///   carries high bits, per [`Lv2Host::narrow_u32_args`].
    /// - `CELL_EINVAL` when the `entry_count` register is no sign
    ///   extension of its low word, per [`Lv2Host::narrow_i32_args`].
    ///
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
        let Some([entry_count]) = self.narrow_i32_args(
            syscall::MMAPPER_ALLOCATE_SHARED_MEMORY_EXT,
            [("entry_count", args[4])],
        ) else {
            return Lv2Dispatch::immediate(errno::CELL_EINVAL.into());
        };
        let flags = u64::from(flags);
        let ipc_key = args[0];
        let size = args[1];
        let handle = match shared_memory_handle(size, flags) {
            Ok(handle) => handle,
            Err(code) => return Lv2Dispatch::immediate(code.into()),
        };
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
        if keyed && self.state.mmapper_ipc.contains_key(ipc_key) {
            return Lv2Dispatch::immediate(errno::CELL_EEXIST.into());
        }
        let mem_id = self.mint_shared_memory(handle);
        if keyed {
            self.state.mmapper_ipc.insert(ipc_key, mem_id);
            if crate::host::is_system_ipc_key(ipc_key) {
                self.obs.system_ipc_witness.shm_creates += 1;
                self.obs.system_ipc_witness.note_key(ipc_key);
            }
        }
        mem_id_written(mem_id_ptr, mem_id, requester, tick)
    }

    /// Mints a `mem_id` and records the handle it names.
    fn mint_shared_memory(&mut self, handle: MmapperHandle) -> u32 {
        let mem_id = self.alloc_id();
        self.state.mmapper_handles.insert(mem_id, handle);
        mem_id
    }
}

/// The handle a shared-memory `size` and `flags` pair describes.
///
/// # Errors
///
/// Listed in the order they fire, the order 332, 339 and 362 share:
///
/// - `CELL_EALIGN` when `size` is zero.
/// - `CELL_EINVAL` when the `flags` granularity field carries an
///   encoding the kernel does not accept.
/// - `CELL_ENOMEM` when `size` does not fit in `u32`.
/// - `CELL_EALIGN` when `size` is not a multiple of the granule the
///   `flags` field selects.
fn shared_memory_handle(size: u64, flags: u64) -> Result<MmapperHandle, Lv2ErrCode> {
    if size == 0 {
        return Err(errno::CELL_EALIGN);
    }
    let Some(align) = accepted_granule(flags) else {
        return Err(errno::CELL_EINVAL);
    };
    let Ok(size) = u32::try_from(size) else {
        return Err(errno::CELL_ENOMEM);
    };
    if !size.is_multiple_of(align) {
        return Err(errno::CELL_EALIGN);
    }
    Ok(MmapperHandle { size, align })
}

/// Success with `mem_id` written to `*mem_id_ptr`.
fn mem_id_written(
    mem_id_ptr: u32,
    mem_id: u32,
    requester: UnitId,
    tick: GuestTicks,
) -> Lv2Dispatch {
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
/// The granularity field is `flags` bits 8 to 11. 64 KiB and 1 MiB are
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
