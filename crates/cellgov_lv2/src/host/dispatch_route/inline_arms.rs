//! Per-arm dispatch helpers for [`super::Lv2Host::dispatch`]'s typed
//! [`Lv2Request`] variants plus the `Unsupported` / `Malformed` /
//! `Hypercall` catch-alls.

use cellgov_effects::{Effect, WritePayload};
use cellgov_event::{PriorityClass, UnitId};
use cellgov_mem::ByteRange;
use cellgov_ps3_abi::cell_errors;

use crate::dispatch::Lv2Dispatch;

use crate::host::{Lv2Host, Lv2Runtime};

impl Lv2Host {
    /// `sys_spu_thread_group_terminate`: SPU teardown is not
    /// modeled; returns CELL_ENOSYS.
    pub(super) fn dispatch_spu_thread_group_terminate_stub(
        &mut self,
        group_id: u32,
        value: i32,
    ) -> Lv2Dispatch {
        self.log_invariant_break(
            "dispatch.spu_thread_group_terminate_stub",
            format_args!(
                "sys_spu_thread_group_terminate(group_id={group_id}, value={value}) \
                 not implemented; returning CELL_ENOSYS"
            ),
        );
        Lv2Dispatch::immediate(cell_errors::CELL_ENOSYS.into())
    }

    /// `sys_memory_free`: bump allocator does not track per-allocation
    /// state, so a valid-free vs bad-pointer vs unknown-id distinction
    /// cannot be made; logged as a known gap and returns CELL_OK.
    pub(super) fn dispatch_memory_free_noop(&mut self) -> Lv2Dispatch {
        self.log_invariant_break(
            "dispatch.memory_free_noop",
            format_args!(
                "sys_memory_free: bump allocator does not reclaim; \
                 returning CELL_OK without state change"
            ),
        );
        Lv2Dispatch::immediate(0u64)
    }

    /// Mints a kernel id and writes it through `*cid_ptr`.
    pub(super) fn dispatch_memory_container_create(
        &mut self,
        cid_ptr: u32,
        requester: UnitId,
    ) -> Lv2Dispatch {
        let id = self.alloc_id();
        self.immediate_write_u32(id, cid_ptr, requester)
    }

    /// `sys_ppu_thread_yield`: round-robin advance happens on the
    /// syscall itself, so the host returns CELL_OK with no effects.
    pub(super) fn dispatch_ppu_thread_yield(&self) -> Lv2Dispatch {
        Lv2Dispatch::immediate(0)
    }

    /// `sys_ppu_thread_start`: no-op CELL_OK.
    ///
    /// Known gap: real LV2 creates threads SUSPENDED and transitions
    /// them here; CellGov collapses both into create.
    pub(super) fn dispatch_ppu_thread_start(&self, _target: u64) -> Lv2Dispatch {
        Lv2Dispatch::immediate(0)
    }

    pub(super) fn dispatch_time_get_timebase_frequency(&self) -> Lv2Dispatch {
        Lv2Dispatch::immediate(cellgov_time::CELL_PPU_TIMEBASE_HZ)
    }

    /// Writes UTC zeros through both out-pointers; EFAULT on null.
    pub(super) fn dispatch_time_get_timezone(
        &self,
        timezone_ptr: u32,
        summer_time_ptr: u32,
        requester: UnitId,
    ) -> Lv2Dispatch {
        if let Some(d) = self.efault_if_null(&[timezone_ptr, summer_time_ptr]) {
            return d;
        }
        let zero = 0i32.to_be_bytes();
        let tz_write = Effect::SharedWriteIntent {
            range: ByteRange::contiguous_u32(timezone_ptr, 4),
            bytes: WritePayload::from_slice(&zero),
            ordering: PriorityClass::Normal,
            source: requester,
            source_time: self.current_tick,
        };
        let dst_write = Effect::SharedWriteIntent {
            range: ByteRange::contiguous_u32(summer_time_ptr, 4),
            bytes: WritePayload::from_slice(&zero),
            ordering: PriorityClass::Normal,
            source: requester,
            source_time: self.current_tick,
        };
        Lv2Dispatch::Immediate {
            code: 0,
            effects: vec![tz_write, dst_write],
        }
    }

    /// Writes `(total, available)` = `(0x0D50_0000, 0x0D50_0000)` to
    /// `*mem_info_ptr` (PS3 game-mode user-memory cap). EFAULT on null.
    pub(super) fn dispatch_memory_get_user_memory_size(
        &self,
        mem_info_ptr: u32,
        requester: UnitId,
    ) -> Lv2Dispatch {
        if let Some(d) = self.efault_if_null(&[mem_info_ptr]) {
            return d;
        }
        let total = cellgov_ps3_abi::sys_memory::USER_MEMORY_TOTAL;
        let available = total;
        let mut bytes = [0u8; 8];
        bytes[0..4].copy_from_slice(&total.to_be_bytes());
        bytes[4..8].copy_from_slice(&available.to_be_bytes());
        let write = Effect::SharedWriteIntent {
            range: ByteRange::contiguous_u32(mem_info_ptr, 8),
            bytes: WritePayload::from_slice(&bytes),
            ordering: PriorityClass::Normal,
            source: requester,
            source_time: self.current_tick,
        };
        Lv2Dispatch::Immediate {
            code: 0,
            effects: vec![write],
        }
    }

    /// Writes `(sec, nsec)` derived from the dispatch-entry tick
    /// snapshot; EFAULT on null.
    pub(super) fn dispatch_time_get_current_time(
        &self,
        sec_ptr: u32,
        nsec_ptr: u32,
        requester: UnitId,
    ) -> Lv2Dispatch {
        if let Some(d) = self.efault_if_null(&[sec_ptr, nsec_ptr]) {
            return d;
        }
        let (sec, nsec) = cellgov_time::ticks_to_sec_nsec(self.current_tick.raw());
        let sec_write = Effect::SharedWriteIntent {
            range: ByteRange::contiguous_u32(sec_ptr, 8),
            bytes: WritePayload::from_slice(&sec.to_be_bytes()),
            ordering: PriorityClass::Normal,
            source: requester,
            source_time: self.current_tick,
        };
        let nsec_write = Effect::SharedWriteIntent {
            range: ByteRange::contiguous_u32(nsec_ptr, 8),
            bytes: WritePayload::from_slice(&nsec.to_be_bytes()),
            ordering: PriorityClass::Normal,
            source: requester,
            source_time: self.current_tick,
        };
        Lv2Dispatch::Immediate {
            code: 0,
            effects: vec![sec_write, nsec_write],
        }
    }

    /// Wraps `dispatch_ppu_thread_create` with a log on nonzero `flags`;
    /// `SYS_PPU_THREAD_CREATE_{JOINABLE,INTERRUPT}` are not modeled.
    #[allow(clippy::too_many_arguments, reason = "mirrors the Lv2Request variant")]
    pub(super) fn dispatch_ppu_thread_create_with_flag_log(
        &mut self,
        id_ptr: u32,
        param_ptr: u32,
        arg: u64,
        priority: i32,
        stacksize: u64,
        flags: u64,
        rt: &dyn Lv2Runtime,
    ) -> Lv2Dispatch {
        if flags != 0 {
            self.log_invariant_break(
                "dispatch.ppu_thread_create_unmodeled_flags",
                format_args!(
                    "sys_ppu_thread_create flags=0x{flags:x} not modeled; \
                     treating as default mode"
                ),
            );
        }
        let priority = priority as u32;
        self.dispatch_ppu_thread_create(id_ptr, param_ptr, arg, priority, stacksize, rt)
    }

    /// `sys_ss_access_control_engine`. Oracle: RPCS3's `sys_ss.cpp`.
    /// `pkg_id` 1/3 require debug-or-root and return ENOSYS for
    /// user-perm callers. `pkg_id == 2` writes the booting process's
    /// program authority id to `*a2` -- boot supplies it from the
    /// title SELF's identification header via
    /// [`Lv2Host::set_program_authority_id`]; raw-ELF inputs serve
    /// the retail-application fallback. Firmware modules classify
    /// callers by this value (libsysmodule's module_start skips its
    /// init entirely for recognized system-process ids), so it must
    /// name the booting title, never a system SELF. Any other
    /// `pkg_id` is SS-domain status `0x8001_051D`.
    pub(super) fn dispatch_ss_access_control_engine(
        &self,
        pkg_id: u64,
        a2: u64,
        requester: UnitId,
    ) -> Lv2Dispatch {
        match pkg_id {
            1 | 3 => Lv2Dispatch::immediate(cell_errors::CELL_ENOSYS.into()),
            2 => match u32::try_from(a2) {
                Err(_) => Lv2Dispatch::immediate(cell_errors::CELL_EFAULT.into()),
                Ok(0) => Lv2Dispatch::immediate(cell_errors::CELL_EFAULT.into()),
                Ok(addr) => {
                    let authid_be = self.program_authority_id.to_be_bytes();
                    let write = Effect::SharedWriteIntent {
                        range: ByteRange::contiguous_u32(addr, 8),
                        bytes: WritePayload::from_slice(&authid_be),
                        ordering: PriorityClass::Normal,
                        source: requester,
                        source_time: self.current_tick,
                    };
                    Lv2Dispatch::Immediate {
                        code: 0,
                        effects: vec![write],
                    }
                }
            },
            _ => Lv2Dispatch::immediate(0x8001_051D),
        }
    }

    /// `sys_timer_create` stub: bumps the `ProcessCounts` timer
    /// counter, mints an id, writes it through `*id_ptr`.
    pub(super) fn dispatch_timer_create(&mut self, id_ptr: u32, requester: UnitId) -> Lv2Dispatch {
        self.process_counts.timer_inc();
        let id = self.alloc_id();
        self.immediate_write_u32(id, id_ptr, requester)
    }

    /// `sys_timer_destroy` stub: decrements the `ProcessCounts`
    /// timer counter and returns CELL_OK.
    pub(super) fn dispatch_timer_destroy(&mut self) -> Lv2Dispatch {
        self.process_counts.timer_dec();
        Lv2Dispatch::immediate(0)
    }

    /// `sys_rwlock_create` stub: mirrors [`Self::dispatch_timer_create`]
    /// against the rwlock counter.
    pub(super) fn dispatch_rwlock_create(&mut self, id_ptr: u32, requester: UnitId) -> Lv2Dispatch {
        self.process_counts.rwlock_inc();
        let id = self.alloc_id();
        self.immediate_write_u32(id, id_ptr, requester)
    }

    /// `sys_rwlock_destroy` stub: mirrors [`Self::dispatch_timer_destroy`].
    pub(super) fn dispatch_rwlock_destroy(&mut self) -> Lv2Dispatch {
        self.process_counts.rwlock_dec();
        Lv2Dispatch::immediate(0)
    }

    /// `sys_event_port_create` stub: mirrors [`Self::dispatch_timer_create`]
    /// against the event-port counter.
    pub(super) fn dispatch_event_port_create(
        &mut self,
        id_ptr: u32,
        requester: UnitId,
    ) -> Lv2Dispatch {
        self.process_counts.event_port_inc();
        let id = self.alloc_id();
        self.immediate_write_u32(id, id_ptr, requester)
    }

    /// `sys_event_port_destroy` stub: mirrors [`Self::dispatch_timer_destroy`].
    pub(super) fn dispatch_event_port_destroy(&mut self) -> Lv2Dispatch {
        self.process_counts.event_port_dec();
        Lv2Dispatch::immediate(0)
    }

    /// PS3 usermode never issues `sc` with LEV != 0; reject with
    /// CELL_EINVAL and log.
    pub(super) fn dispatch_hypercall_rejection(
        &mut self,
        lev: u8,
        r11: u64,
        args: [u64; 8],
    ) -> Lv2Dispatch {
        self.log_invariant_break(
            "dispatch.hypercall_rejected",
            format_args!(
                "sc LEV={lev} r11={r11:#x} from PS3 usermode; \
                 hypercalls are a programming error \
                 (r3={:#x} r4={:#x} r5={:#x} r6={:#x} r7={:#x} r8={:#x} r9={:#x} r10={:#x})",
                args[0], args[1], args[2], args[3], args[4], args[5], args[6], args[7],
            ),
        );
        Lv2Dispatch::immediate(cell_errors::CELL_EINVAL.into())
    }

    /// `Unsupported` catch-all: log and return CELL_ENOSYS.
    pub(super) fn dispatch_unsupported_default(
        &mut self,
        number: u64,
        args: [u64; 8],
    ) -> Lv2Dispatch {
        self.log_invariant_break(
            "dispatch.unsupported_stub",
            format_args!(
                "syscall {number} has no dispatch handler (r3={:#x} r4={:#x} r5={:#x} \
                 r6={:#x} r7={:#x} r8={:#x} r9={:#x} r10={:#x}); returning CELL_ENOSYS",
                args[0], args[1], args[2], args[3], args[4], args[5], args[6], args[7],
            ),
        );
        Lv2Dispatch::immediate(cell_errors::CELL_ENOSYS.into())
    }

    /// `Malformed` rejection: classifier failed to bind request fields;
    /// log and return CELL_EINVAL.
    pub(super) fn dispatch_malformed_rejection(
        &mut self,
        number: u64,
        reason: &'static str,
        args: [u64; 8],
    ) -> Lv2Dispatch {
        self.log_invariant_break(
            "dispatch.malformed_syscall",
            format_args!(
                "syscall {number} rejected: {reason} (r3={:#x} r4={:#x} r5={:#x} \
                 r6={:#x} r7={:#x} r8={:#x} r9={:#x} r10={:#x})",
                args[0], args[1], args[2], args[3], args[4], args[5], args[6], args[7],
            ),
        );
        Lv2Dispatch::immediate(cell_errors::CELL_EINVAL.into())
    }

    /// `UnresolvedImport`: trampoline in an unpatched GOT slot fired;
    /// log NID + name (if in the db) and return CELL_EINVAL.
    pub(super) fn dispatch_unresolved_import(
        &mut self,
        nid: u32,
        _requester: cellgov_event::UnitId,
    ) -> Lv2Dispatch {
        match cellgov_ps3_abi::nid::lookup(nid) {
            Some((module, name)) => {
                let module_label = if module.is_empty() {
                    "<unknown>"
                } else {
                    module
                };
                self.log_invariant_break(
                    "dispatch.unresolved_import",
                    format_args!(
                        "GOT slot for NID 0x{nid:08x} ({module_label}::{name}) was not bound \
                         by patch_got_atomic; returning CELL_EINVAL",
                    ),
                );
            }
            None => {
                self.log_invariant_break(
                    "dispatch.unresolved_import",
                    format_args!(
                        "GOT slot for NID 0x{nid:08x} (no name in NID db) was not bound by \
                         patch_got_atomic; returning CELL_EINVAL",
                    ),
                );
            }
        }
        Lv2Dispatch::immediate(cell_errors::CELL_EINVAL.into())
    }
}
