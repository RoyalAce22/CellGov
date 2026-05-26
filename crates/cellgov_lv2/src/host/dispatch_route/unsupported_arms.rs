//! Per-syscall dispatch helpers for the specific `Lv2Request::Unsupported
//! { number: N }` arms. Each method handles one syscall number; the
//! match in `mod.rs` reduces to a one-line delegation per number.

use cellgov_effects::{Effect, WritePayload};
use cellgov_event::{PriorityClass, UnitId};
use cellgov_mem::ByteRange;
use cellgov_ps3_abi::cell_errors as errno;
use cellgov_ps3_abi::sys_memory::page_size;

use crate::dispatch::Lv2Dispatch;

use crate::host::mmapper::{MmapperHandle, PendingRegionInstall};
use crate::host::{Lv2Host, Lv2Runtime};

impl Lv2Host {
    /// `sys_ppu_thread_get_priority` (48). Oracle:
    /// `rpcs3/Emu/Cell/lv2/sys_ppu_thread.cpp:365` writes the
    /// target thread's priority (s32) to `*priop`. Unknown thread
    /// ids fall back to 1001, the boot-seed primary thread priority,
    /// so firmware-side callers that read back the value see a
    /// self-consistent answer.
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
    /// not modeled. RPCS3 implements the binding and returns CELL_OK
    /// after persisting it
    /// (`tools/rpcs3-src/rpcs3/Emu/Cell/lv2/sys_event.cpp:666-694`);
    /// returning CELL_OK without the binding would let the guest
    /// proceed believing sends will deliver when they would silently
    /// vanish. CELL_ENOSYS is the honest divergent gap.
    pub(super) fn dispatch_event_port_connect_local(&mut self) -> Lv2Dispatch {
        self.log_invariant_break(
            "dispatch.event_port_connect_local_unmodeled",
            format_args!(
                "sys_event_port_connect_local: port -> queue binding not modeled; \
                 returning CELL_ENOSYS"
            ),
        );
        Lv2Dispatch::immediate(errno::CELL_ENOSYS.into())
    }

    /// `sys_memory_container_create` (324). Oracle:
    /// `rpcs3/Emu/Cell/lv2/sys_memory.cpp:375` mints an
    /// `lv2_memory_container` id and writes it to `*cid`. The oracle
    /// does not track physical-memory budgets. Suffix `_324`
    /// disambiguates this Unsupported{N} arm from the typed
    /// `MemoryContainerCreate` variant handled by
    /// [`Lv2Host::dispatch_memory_container_create`].
    pub(super) fn dispatch_memory_container_create_324(
        &mut self,
        args: [u64; 8],
        requester: UnitId,
    ) -> Lv2Dispatch {
        let cid_ptr = args[0] as u32;
        if let Some(d) = self.efault_if_null(&[cid_ptr]) {
            return d;
        }
        // SYS_MEMORY_CONTAINER_OBJECT has no `count_for_class`
        // arm, so an inc here would be unobserved dead state.
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

    /// `sys_mmapper_allocate_address` (330). Oracle:
    /// `rpcs3/Emu/Cell/lv2/sys_mmapper.cpp:130` validates size as a
    /// 256 MiB multiple, defaults alignment 0 to 256 MiB, and writes
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
            None => Lv2Dispatch::immediate(errno::CELL_ENOMEM.into()),
        }
    }

    /// `sys_mmapper_allocate_shared_memory` (332). Oracle:
    /// `rpcs3/Emu/Cell/lv2/sys_mmapper.cpp:190` creates an `lv2_memory`
    /// shm object and writes the u32 id to `*mem_id_ptr`. The oracle
    /// mints a monotonic id; the map / search-and-map calls that
    /// follow are no-ops at the oracle layer.
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
            return Lv2Dispatch::immediate(errno::CELL_ENOMEM.into());
        };
        let align = page_size::granule_from_flags(flags);
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
    /// Oracle: `tools/rpcs3-src/rpcs3/Emu/Cell/lv2/sys_mmapper.cpp:613-686`.
    /// RPCS3 bounds `addr` to `[0x2000_0000, 0xC000_0000)`, looks the
    /// shm handle up via `idm::get`, checks `addr % page_alignment`, then
    /// `vm::falloc`s the pages.
    ///
    /// # Errors
    ///
    /// - `CELL_EINVAL` for `addr < 0x2000_0000 || addr >= 0xC000_0000`.
    /// - `CELL_ESRCH` (plus a `dispatch.mmapper_map_unknown_mem_id`
    ///   invariant break) when `mem_id` is not in the handle table.
    /// - `CELL_EALIGN` when `addr % handle.align != 0`.
    /// - `CELL_EBUSY` (plus `dispatch.mmapper_map_overlap` break) when
    ///   the requested range overlaps an existing region.
    ///
    /// The Phase-37 "writable spot-check" fast path stays: if `addr`
    /// already lies in a backed region (genuine main / iomap / stack
    /// target, e.g. a synthetic-ELF harness's 334 against a flat backing)
    /// the call returns CELL_OK without recording state. This preserves
    /// existing test fixtures while letting the handle path do the
    /// real work for firmware-set boots.
    pub(super) fn dispatch_mmapper_map_shared_memory(
        &mut self,
        args: [u64; 8],
        rt: &dyn Lv2Runtime,
    ) -> Lv2Dispatch {
        let addr = args[0];
        let mem_id = args[1] as u32;
        // Flat-backing fast path: an already-backed `addr` short-circuits
        // to CELL_OK regardless of `mem_id`. Used by synthetic ELFs that
        // 334 against pre-mapped main memory; firmware-set boots take
        // the handle-keyed path below.
        if rt.writable(addr, 1) {
            return Lv2Dispatch::immediate(errno::CELL_OK.into());
        }
        if !(0x2000_0000..0xC000_0000).contains(&addr) {
            return Lv2Dispatch::immediate(errno::CELL_EINVAL.into());
        }
        let Some(handle) = self.mmapper_handles.get(mem_id) else {
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
        // Bound the install at u32 so overflow-into-MMIO is rejected
        // here rather than at the runtime's install_region check.
        let Some(end) = addr.checked_add(u64::from(handle.size)) else {
            return Lv2Dispatch::immediate(errno::CELL_EINVAL.into());
        };
        if end > 0xC000_0000 {
            return Lv2Dispatch::immediate(errno::CELL_EINVAL.into());
        }
        self.pending_region_installs.push(PendingRegionInstall {
            addr,
            size: handle.size as usize,
        });
        Lv2Dispatch::immediate(errno::CELL_OK.into())
    }

    /// `sys_mmapper_search_and_map` (337). Oracle:
    /// `rpcs3/Emu/Cell/lv2/sys_mmapper.cpp:688` validates `start_addr`
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
            return Lv2Dispatch::immediate(errno::CELL_EINVAL.into());
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
    /// Oracle: `rpcs3/Emu/Cell/lv2/sys_mmapper.cpp:242`. Same shape
    /// as syscall 332 with a caller-supplied container; the
    /// out-pointer for the fresh mem_id is at r7.
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
            return Lv2Dispatch::immediate(errno::CELL_ENOMEM.into());
        };
        let align = page_size::granule_from_flags(flags);
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
        Lv2Dispatch::immediate(errno::CELL_EIO.into())
    }

    /// DEX-only slot (462). `uns_func` in
    /// `rpcs3/Emu/Cell/lv2/lv2.cpp:511`; retail liblv2 expects ENOSYS
    /// to take its fallback path.
    pub(super) fn dispatch_uns_func_462(&self) -> Lv2Dispatch {
        Lv2Dispatch::immediate(errno::CELL_ENOSYS.into())
    }

    /// `_sys_prx_start_module` (481). Oracle:
    /// `rpcs3/Emu/Cell/lv2/sys_prx.cpp:590` writes
    /// `pOpt->entry = prx->start ? prx->start.addr() : ~0` before
    /// returning CELL_OK. Struct layout per
    /// `rpcs3/Emu/Cell/lv2/sys_prx.h:107` puts `entry` at offset 16.
    /// `~0` is the kernel sentinel for "no start function".
    /// `if (!id || !pOpt || pOpt->size < 0x20) return CELL_EINVAL;`
    /// is honored for id and pOpt; the size check is deferred.
    pub(super) fn dispatch_prx_start_module(
        &self,
        args: [u64; 8],
        requester: UnitId,
    ) -> Lv2Dispatch {
        let id = args[0] as u32;
        let p_opt = args[2] as u32;
        if id == 0 || p_opt == 0 {
            return Lv2Dispatch::immediate(errno::CELL_EINVAL.into());
        }
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

    /// `_sys_prx_register_module` (484). Oracle:
    /// `rpcs3/Emu/Cell/lv2/sys_prx.cpp:860` returns
    /// CELL_PRX_ERROR_ELF_IS_REGISTERED for non-VSH callers (wrapped
    /// in `not_an_error`).
    pub(super) fn dispatch_prx_register_module(&self) -> Lv2Dispatch {
        Lv2Dispatch::immediate(0x8001_1910)
    }

    /// `_sys_prx_register_library` (486). Oracle:
    /// `rpcs3/Emu/Cell/lv2/sys_prx.cpp:875` walks every PRX's export
    /// table for a match. With no firmware-side import resolution
    /// modeled, CELL_OK matches the kernel's "no match" success path.
    pub(super) fn dispatch_prx_register_library(&self) -> Lv2Dispatch {
        Lv2Dispatch::immediate(0)
    }

    /// `_sys_prx_get_module_list` (494). Oracle:
    /// `rpcs3/Emu/Cell/lv2/sys_prx.cpp:954`. `flags & 0x2 == 0`
    /// short-circuits to CELL_OK; otherwise the kernel walks every
    /// loaded lv2_prx (filtering liblv2.sprx) and fills
    /// `pInfo->idlist` up to `pInfo->max`, then writes `pInfo->count`.
    /// Struct layout per `rpcs3/Emu/Cell/lv2/sys_prx.h:129`:
    /// `size@0, pad@8, max@0xC, count@0x10, idlist@0x14, unk@0x1C`.
    /// CELL_EFAULT on null `pInfo` when bit 2 is set.
    pub(super) fn dispatch_prx_get_module_list(
        &self,
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
            return Lv2Dispatch::immediate(errno::CELL_EFAULT.into());
        }
        let mut effects = Vec::new();
        let max_addr = p_info.wrapping_add(0x0C);
        let count_addr = p_info.wrapping_add(0x10);
        let idlist_ptr_addr = p_info.wrapping_add(0x14);
        let max = rt
            .read_committed(u64::from(max_addr), 4)
            .map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
            .unwrap_or(0);
        let idlist_ptr = rt
            .read_committed(u64::from(idlist_ptr_addr), 4)
            .map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
            .unwrap_or(0);
        let liblv2_id = self
            .prx_registry
            .lookup_by_path("liblv2.sprx")
            .map(|e| e.kernel_id());
        let mut count: u32 = 0;
        if idlist_ptr != 0 {
            // `prx_registry.ids()` iterates `BTreeMap` keys in
            // monotonic kernel-id order; idlist bytes are therefore
            // independent of registry insertion order.
            for kid in self.prx_registry.ids() {
                if Some(kid) == liblv2_id {
                    continue;
                }
                if count >= max {
                    break;
                }
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
    /// `rpcs3/Emu/Cell/lv2/sys_hid.cpp:140` returns the caller's
    /// root bit. Retail titles run unprivileged (false).
    pub(super) fn dispatch_hid_is_root(&self) -> Lv2Dispatch {
        Lv2Dispatch::immediate(0)
    }

    /// `sys_gamepad_ycon_if` (621). Convergent honest gap: RPCS3's
    /// implementation is also a stub -- every packet_id sub-handler
    /// logs `todo()` and returns CELL_OK; the unknown-packet
    /// default also returns CELL_OK
    /// (`tools/rpcs3-src/rpcs3/Emu/Cell/lv2/sys_gamepad.cpp:7-98`).
    /// CellGov matches that shape; the diagnostic fires so the
    /// stub is traced.
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

    /// `sys_rsx_attribute` (677). Oracle:
    /// `rpcs3/Emu/Cell/lv2/sys_rsx.cpp:983` logs and returns CELL_OK
    /// without state change.
    pub(super) fn dispatch_rsx_attribute(&self) -> Lv2Dispatch {
        Lv2Dispatch::immediate(0)
    }
}
