//! Per-syscall dispatch helpers for the `Lv2Request::Unsupported
//! { number: N }` arms; one method per syscall number.

use cellgov_effects::{Effect, WritePayload};
use cellgov_event::{PriorityClass, UnitId};
use cellgov_mem::ByteRange;
use cellgov_ps3_abi::cell_errors;
use cellgov_ps3_abi::sys_memory::page_size;

use crate::dispatch::Lv2Dispatch;

use crate::host::mmapper::{MmapperHandle, PendingRegionInstall};
use crate::host::{Lv2Host, Lv2Runtime};

impl Lv2Host {
    /// `sys_ppu_thread_get_priority` (48). Oracle: RPCS3's
    /// `sys_ppu_thread.cpp` writes the target thread's priority (s32)
    /// to `*priop`. Unknown thread ids fall back to 1001 (boot-seed
    /// primary priority) for a self-consistent read-back.
    pub(super) fn dispatch_ppu_thread_get_priority(
        &self,
        args: [u64; 8],
        requester: UnitId,
    ) -> Lv2Dispatch {
        let thread_id = args[0] as u32;
        let priop = args[1] as u32;
        if let Some(d) = self.efault_if_null(&[priop]) {
            return d;
        }
        let priority = self
            .ppu_threads
            .get(crate::ppu_thread::PpuThreadId::new(thread_id as u64))
            .map(|t| t.attrs.priority)
            .unwrap_or(1001);
        let write = Effect::SharedWriteIntent {
            range: ByteRange::contiguous_u32(priop, 4),
            bytes: WritePayload::from_slice(&priority.to_be_bytes()),
            ordering: PriorityClass::Normal,
            source: requester,
            source_time: self.current_tick,
        };
        Lv2Dispatch::Immediate {
            code: 0,
            effects: vec![write],
        }
    }

    /// `sys_event_port_connect_local` (136). Port -> queue binding is
    /// not modeled; honest gap reports CELL_ENOSYS rather than RPCS3's
    /// CELL_OK without a persisted binding.
    pub(super) fn dispatch_event_port_connect_local(&mut self) -> Lv2Dispatch {
        self.log_invariant_break(
            "dispatch.event_port_connect_local_unmodeled",
            format_args!(
                "sys_event_port_connect_local: port -> queue binding not modeled; \
                 returning CELL_ENOSYS"
            ),
        );
        Lv2Dispatch::immediate(cell_errors::CELL_ENOSYS.into())
    }

    /// `sys_memory_container_create` (324). Oracle: RPCS3's
    /// `sys_memory.cpp` mints an `lv2_memory_container` id and writes
    /// it to `*cid`. Physical-memory budgets are not tracked.
    pub(super) fn dispatch_memory_container_create_324(
        &mut self,
        args: [u64; 8],
        requester: UnitId,
    ) -> Lv2Dispatch {
        let cid_ptr = args[0] as u32;
        if let Some(d) = self.efault_if_null(&[cid_ptr]) {
            return d;
        }
        let cid = self.alloc_id();
        let write = Effect::SharedWriteIntent {
            range: ByteRange::contiguous_u32(cid_ptr, 4),
            bytes: WritePayload::from_slice(&cid.to_be_bytes()),
            ordering: PriorityClass::Normal,
            source: requester,
            source_time: self.current_tick,
        };
        Lv2Dispatch::Immediate {
            code: 0,
            effects: vec![write],
        }
    }

    /// `sys_mmapper_allocate_address` (330). Oracle: RPCS3's
    /// `sys_mmapper.cpp` validates size as a 256 MiB multiple,
    /// defaults alignment 0 to 256 MiB, and writes
    /// a free VM-area base to `*alloc_addr`. The oracle bumps a
    /// monotonic 256 MiB-aligned cursor; overflow returns CELL_ENOMEM.
    pub(super) fn dispatch_mmapper_allocate_address(
        &mut self,
        args: [u64; 8],
        requester: UnitId,
    ) -> Lv2Dispatch {
        let size = args[0] as u32;
        let alloc_addr_ptr = args[3] as u32;
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
                    source_time: self.current_tick,
                };
                Lv2Dispatch::Immediate {
                    code: 0,
                    effects: vec![write],
                }
            }
            None => Lv2Dispatch::immediate(cell_errors::CELL_ENOMEM.into()),
        }
    }

    /// `sys_mmapper_allocate_shared_memory` (332). Oracle: RPCS3's
    /// `sys_mmapper.cpp` creates an `lv2_memory` shm object and
    /// writes the u32 id to `*mem_id_ptr`. The oracle
    /// mints a monotonic id; the map / search-and-map calls that
    /// follow are no-ops at the oracle layer.
    ///
    /// # Errors
    ///
    /// - `CELL_EFAULT` when `mem_id_ptr` is null.
    /// - `CELL_ENOMEM` when `size` does not fit in `u32`.
    /// - `CELL_EALIGN` when `size % granule_from_flags(flags) != 0`.
    pub(super) fn dispatch_mmapper_allocate_shared_memory(
        &mut self,
        args: [u64; 8],
        requester: UnitId,
    ) -> Lv2Dispatch {
        let size = args[1];
        let flags = args[2];
        let mem_id_ptr = args[3] as u32;
        if let Some(d) = self.efault_if_null(&[mem_id_ptr]) {
            return d;
        }
        let Ok(size_u32) = u32::try_from(size) else {
            return Lv2Dispatch::immediate(cell_errors::CELL_ENOMEM.into());
        };
        let align = page_size::granule_from_flags(flags);
        if !size_u32.is_multiple_of(align) {
            return Lv2Dispatch::immediate(cell_errors::CELL_EALIGN.into());
        }
        let mem_id = self.alloc_id();
        self.mmapper_handles.insert(
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
            source_time: self.current_tick,
        };
        Lv2Dispatch::Immediate {
            code: 0,
            effects: vec![write],
        }
    }

    /// `sys_mmapper_map_shared_memory` (334). Looks the caller's
    /// `mem_id` up in the handle table populated by 332 / 362, validates
    /// `addr` against the handle's recorded page granule, and pushes a
    /// pending region-install request the runtime drains after dispatch.
    ///
    /// Oracle: RPCS3's `sys_mmapper.cpp` map handler. It bounds `addr`
    /// to `[0x2000_0000, 0xC000_0000)`, looks the shm handle up via
    /// `idm::get`, checks `addr % page_alignment`, then `vm::falloc`s
    /// the pages.
    ///
    /// # Errors
    ///
    /// - `CELL_EINVAL` for `addr < 0x2000_0000 || addr >= 0xC000_0000`
    ///   or for `addr + handle.size` past `0xC000_0000`.
    /// - `CELL_ESRCH` (plus a `dispatch.mmapper_map_unknown_mem_id`
    ///   invariant break) when `mem_id` is not in the handle table.
    /// - `CELL_EALIGN` when `addr % handle.align != 0`.
    ///
    /// Overlap with an existing region is not detected here. The
    /// runtime's `install_region` drain rejects an overlapping
    /// `PendingRegionInstall`, which surfaces as a dispatch panic
    /// rather than a per-call errno -- the handle-table state is the
    /// invariant the dispatch path must maintain.
    pub(super) fn dispatch_mmapper_map_shared_memory(&mut self, args: [u64; 8]) -> Lv2Dispatch {
        let addr = args[0];
        let mem_id = args[1] as u32;
        if !(0x2000_0000..0xC000_0000).contains(&addr) {
            return Lv2Dispatch::immediate(cell_errors::CELL_EINVAL.into());
        }
        let Some(handle) = self.mmapper_handles.get(mem_id) else {
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
        // Reject overflow-into-MMIO here so the runtime's install_region
        // check never sees an out-of-range request.
        let Some(end) = addr.checked_add(u64::from(handle.size)) else {
            return Lv2Dispatch::immediate(cell_errors::CELL_EINVAL.into());
        };
        if end > 0xC000_0000 {
            return Lv2Dispatch::immediate(cell_errors::CELL_EINVAL.into());
        }
        self.pending_region_installs.push(PendingRegionInstall {
            addr,
            size: handle.size as usize,
        });
        Lv2Dispatch::immediate(cell_errors::CELL_OK.into())
    }

    /// `sys_mmapper_search_and_map` (337). Oracle: RPCS3's
    /// `sys_mmapper.cpp` search-and-map handler validates `start_addr`
    /// within `[0x2000_0000, 0xC000_0000)` and writes the placement
    /// to `*alloc_addr_ptr`. The oracle's flat backing collapses the
    /// search to "place at start_addr"; out-of-range `start_addr`
    /// returns CELL_EINVAL.
    pub(super) fn dispatch_mmapper_search_and_map(
        &self,
        args: [u64; 8],
        requester: UnitId,
    ) -> Lv2Dispatch {
        let start_addr = args[0] as u32;
        let alloc_addr_ptr = args[3] as u32;
        if let Some(d) = self.efault_if_null(&[alloc_addr_ptr]) {
            return d;
        }
        if !(0x2000_0000..0xC000_0000).contains(&start_addr) {
            return Lv2Dispatch::immediate(cell_errors::CELL_EINVAL.into());
        }
        let write = Effect::SharedWriteIntent {
            range: ByteRange::contiguous_u32(alloc_addr_ptr, 4),
            bytes: WritePayload::from_slice(&start_addr.to_be_bytes()),
            ordering: PriorityClass::Normal,
            source: requester,
            source_time: self.current_tick,
        };
        Lv2Dispatch::Immediate {
            code: 0,
            effects: vec![write],
        }
    }

    /// `sys_mmapper_allocate_shared_memory_from_container` (362).
    /// Oracle: RPCS3's `sys_mmapper.cpp` shm-from-container handler;
    /// same shape as syscall 332 with a caller-supplied container;
    /// the out-pointer for the fresh mem_id is at r7. Flags are at r6
    /// (`args[3]`), one slot deeper than 332's r5, because r5 carries
    /// the container id.
    ///
    /// # Errors
    ///
    /// - `CELL_EFAULT` when `mem_id_ptr` is null.
    /// - `CELL_ENOMEM` when `size` does not fit in `u32`.
    /// - `CELL_EALIGN` when `size % granule_from_flags(flags) != 0`.
    pub(super) fn dispatch_mmapper_allocate_shared_memory_from_container(
        &mut self,
        args: [u64; 8],
        requester: UnitId,
    ) -> Lv2Dispatch {
        let size = args[1];
        let flags = args[3];
        let mem_id_ptr = args[4] as u32;
        if let Some(d) = self.efault_if_null(&[mem_id_ptr]) {
            return d;
        }
        let Ok(size_u32) = u32::try_from(size) else {
            return Lv2Dispatch::immediate(cell_errors::CELL_ENOMEM.into());
        };
        let align = page_size::granule_from_flags(flags);
        if !size_u32.is_multiple_of(align) {
            return Lv2Dispatch::immediate(cell_errors::CELL_EALIGN.into());
        }
        let mem_id = self.alloc_id();
        self.mmapper_handles.insert(
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
            source_time: self.current_tick,
        };
        Lv2Dispatch::Immediate {
            code: 0,
            effects: vec![write],
        }
    }

    /// `sys_tty_read` (402): CELL_OK spins CRT input loops forever;
    /// real LV2 returns EIO outside debug console mode.
    pub(super) fn dispatch_tty_read(&self) -> Lv2Dispatch {
        Lv2Dispatch::immediate(cell_errors::CELL_EIO.into())
    }

    /// DEX-only slot (462). `uns_func` in RPCS3's `lv2.cpp` dispatch
    /// table; retail liblv2 expects ENOSYS to take its fallback path.
    pub(super) fn dispatch_uns_func_462(&self) -> Lv2Dispatch {
        Lv2Dispatch::immediate(cell_errors::CELL_ENOSYS.into())
    }

    /// `_sys_prx_start_module` (481). Oracle: RPCS3's `sys_prx.cpp`
    /// writes `pOpt->entry = prx->start ? prx->start.addr() : ~0`
    /// before returning CELL_OK. Struct layout per RPCS3's
    /// `sys_prx.h` puts `size` at offset 0 and `entry` at offset 16.
    /// `~0` is the kernel sentinel for "no start function". RPCS3's
    /// `if (!id || !pOpt || pOpt->size < 0x20) return CELL_EINVAL;`
    /// is honored in full: an unreadable `pOpt->size` faults with
    /// CELL_EFAULT plus a `dispatch.prx_start_module_size_unreadable`
    /// break, and `size < 0x20` returns CELL_EINVAL.
    pub(super) fn dispatch_prx_start_module(
        &mut self,
        args: [u64; 8],
        requester: UnitId,
        rt: &dyn Lv2Runtime,
    ) -> Lv2Dispatch {
        let id = args[0] as u32;
        let p_opt = args[2] as u32;
        if id == 0 || p_opt == 0 {
            return Lv2Dispatch::immediate(cell_errors::CELL_EINVAL.into());
        }
        let Some(size_bytes) = rt.read_committed(u64::from(p_opt), 4) else {
            self.log_invariant_break(
                "dispatch.prx_start_module_size_unreadable",
                format_args!(
                    "_sys_prx_start_module pOpt={p_opt:#010x} size field unreadable; \
                     returning CELL_EFAULT"
                ),
            );
            return Lv2Dispatch::immediate(cell_errors::CELL_EFAULT.into());
        };
        let size = u32::from_be_bytes([size_bytes[0], size_bytes[1], size_bytes[2], size_bytes[3]]);
        if size < 0x20 {
            return Lv2Dispatch::immediate(cell_errors::CELL_EINVAL.into());
        }
        // The 8-byte entry write spans `[p_opt+16, p_opt+24)`. The
        // `size >= 0x20` check above proves the guest declared at
        // least 32 bytes at `p_opt`, so the entry slot is in-struct
        // by guest contract; a wrap here means the guest lied or
        // we miscomputed.
        debug_assert!(
            p_opt.checked_add(24).is_some(),
            "_sys_prx_start_module entry write wraps u32: p_opt={p_opt:#010x}",
        );
        let entry_addr = p_opt.wrapping_add(16);
        let no_start = u64::MAX.to_be_bytes();
        let entry_write = Effect::SharedWriteIntent {
            range: ByteRange::contiguous_u32(entry_addr, 8),
            bytes: WritePayload::from_slice(&no_start),
            ordering: PriorityClass::Normal,
            source: requester,
            source_time: self.current_tick,
        };
        Lv2Dispatch::Immediate {
            code: 0,
            effects: vec![entry_write],
        }
    }

    /// `_sys_prx_register_module` (484). Oracle: RPCS3's `sys_prx.cpp`
    /// returns CELL_PRX_ERROR_ELF_IS_REGISTERED for non-VSH callers
    /// (wrapped in `not_an_error`).
    pub(super) fn dispatch_prx_register_module(&self) -> Lv2Dispatch {
        Lv2Dispatch::immediate(0x8001_1910)
    }

    /// `_sys_prx_register_library` (486). Oracle: RPCS3's `sys_prx.cpp`
    /// walks every PRX's export table for a match. With no
    /// firmware-side import resolution modeled, CELL_OK matches the
    /// kernel's "no match" success path.
    pub(super) fn dispatch_prx_register_library(&self) -> Lv2Dispatch {
        Lv2Dispatch::immediate(0)
    }

    /// `_sys_prx_get_module_list` (494). Oracle: RPCS3's `sys_prx.cpp`.
    /// `flags & 0x2 == 0` short-circuits to CELL_OK; otherwise the
    /// kernel walks every loaded lv2_prx (filtering liblv2.sprx) and
    /// fills `pInfo->idlist` up to `pInfo->max`, then writes
    /// `pInfo->count`. Struct layout per RPCS3's `sys_prx.h`:
    /// `size@0, pad@8, max@0xC, count@0x10, idlist@0x14, unk@0x1C`.
    /// CELL_EFAULT on null `pInfo` when bit 2 is set.
    ///
    /// The idlist slot writes and the trailing count write are emitted
    /// in a single `Lv2Dispatch::Immediate` batch. RPCS3's handler
    /// writes count AFTER the fill loop completes, and both stores go
    /// through the same `vm::ptr<>` access path; an unwritable idlist
    /// page would trap during the loop and prevent the post-loop count
    /// store. The atomic-commit contract enforces the same outcome
    /// here: if any slot intent fails validation, the whole batch
    /// rolls back and count cannot land independently.
    pub(super) fn dispatch_prx_get_module_list(
        &mut self,
        args: [u64; 8],
        requester: UnitId,
        rt: &dyn Lv2Runtime,
    ) -> Lv2Dispatch {
        let flags = args[0];
        let p_info = args[1] as u32;
        if flags & 0x2 == 0 {
            return Lv2Dispatch::immediate(0);
        }
        if p_info == 0 {
            return Lv2Dispatch::immediate(cell_errors::CELL_EFAULT.into());
        }
        // The struct fields span `[p_info, p_info+0x18)`. A wrap here
        // would silently retarget the field reads / writes elsewhere
        // in low memory; an in-range read failure is the recoverable
        // path (handled below), a u32 wrap is not.
        debug_assert!(
            p_info.checked_add(0x18).is_some(),
            "sys_prx_get_module_list pInfo struct wraps u32: pInfo={p_info:#010x}",
        );
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
            .prx_registry
            .lookup_by_path("liblv2.sprx")
            .map(|e| e.kernel_id());
        let mut count: u32 = 0;
        if idlist_ptr != 0 {
            // `ids()` walks `BTreeMap` keys in monotonic id order, so
            // idlist bytes are independent of insertion order.
            for kid in self.prx_registry.ids() {
                if Some(kid) == liblv2_id {
                    continue;
                }
                if count >= max {
                    break;
                }
                // Each slot is a 4-byte write at `idlist_ptr + count*4`;
                // the highest address touched is `slot + 3`. A wrap
                // would retarget the write to low memory unnoticed.
                debug_assert!(
                    count
                        .checked_mul(4)
                        .and_then(|off| idlist_ptr.checked_add(off))
                        .and_then(|s| s.checked_add(4))
                        .is_some(),
                    "sys_prx_get_module_list idlist slot wraps u32: \
                     idlist_ptr={idlist_ptr:#010x} count={count}",
                );
                let slot = idlist_ptr.wrapping_add(count.wrapping_mul(4));
                effects.push(Effect::SharedWriteIntent {
                    range: ByteRange::contiguous_u32(slot, 4),
                    bytes: WritePayload::from_slice(&kid.to_be_bytes()),
                    ordering: PriorityClass::Normal,
                    source: requester,
                    source_time: self.current_tick,
                });
                count += 1;
            }
        }
        effects.push(Effect::SharedWriteIntent {
            range: ByteRange::contiguous_u32(count_addr, 4),
            bytes: WritePayload::from_slice(&count.to_be_bytes()),
            ordering: PriorityClass::Normal,
            source: requester,
            source_time: self.current_tick,
        });
        Lv2Dispatch::Immediate { code: 0, effects }
    }

    /// `sys_hid_manager_is_process_permission_root` (512). Oracle:
    /// RPCS3's `sys_hid.cpp` returns the caller's root bit. Retail
    /// titles run unprivileged (false).
    pub(super) fn dispatch_hid_is_root(&self) -> Lv2Dispatch {
        Lv2Dispatch::immediate(0)
    }

    /// `sys_gamepad_ycon_if` (621). Convergent honest gap: RPCS3's
    /// `sys_gamepad.cpp` is also a CELL_OK stub for every packet_id.
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

    /// `sys_rsx_attribute` (677). Oracle: RPCS3's `sys_rsx.cpp` logs
    /// and returns CELL_OK without state change.
    pub(super) fn dispatch_rsx_attribute(&self) -> Lv2Dispatch {
        Lv2Dispatch::immediate(0)
    }
}
