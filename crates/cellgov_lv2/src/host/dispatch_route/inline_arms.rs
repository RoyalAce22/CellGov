//! Per-arm dispatch helpers extracted from the typed-variant match
//! arms in [`super::Lv2Host::dispatch`]. Each method handles one
//! [`Lv2Request`] variant (or the catch-all `Unsupported` /
//! `Malformed` / `Hypercall` arms); the dispatch match in
//! `mod.rs` reduces to a one-line delegation per arm.

use cellgov_effects::{Effect, WritePayload};
use cellgov_event::{PriorityClass, UnitId};
use cellgov_mem::ByteRange;
use cellgov_ps3_abi::cell_errors as errno;

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
        Lv2Dispatch::immediate(errno::CELL_ENOSYS.into())
    }

    /// `sys_memory_free`: no dealloc tracking. The honest answer
    /// depends on whether the caller's `start_addr` is a valid prior
    /// allocation -- real LV2 returns CELL_OK on a valid free,
    /// CELL_EINVAL on a bad pointer, CELL_ESRCH on an unknown id.
    /// CellGov's bump allocator does not track per-allocation state,
    /// so it cannot distinguish the cases; a blanket ENOSYS would
    /// itself be a lie (allocation works). The interim is to log
    /// every free as a known model gap: the no-op-with-trace is a
    /// convergent honest gap as long as no title in the corpus
    /// frees, re-allocates expecting reuse, and observes the
    /// difference. A proper fix requires per-allocation lifecycle
    /// tracking.
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

    /// Mints a kernel id and writes it through `*cid_ptr`. The
    /// matching `Unsupported { number: 324 }` arm carries the same
    /// shape -- see also [`Lv2Host::immediate_write_u32`] for the
    /// null-pointer guard. No ProcessCounts increment:
    /// `SYS_MEMORY_CONTAINER_OBJECT` has no `count_for_class` arm,
    /// so an inc here would be unobserved dead state.
    pub(super) fn dispatch_memory_container_create(
        &mut self,
        cid_ptr: u32,
        requester: UnitId,
    ) -> Lv2Dispatch {
        let id = self.alloc_id();
        self.immediate_write_u32(id, cid_ptr, requester)
    }

    /// The round-robin walk advances on the syscall yield itself,
    /// so the host has nothing further to do.
    pub(super) fn dispatch_ppu_thread_yield(&self) -> Lv2Dispatch {
        Lv2Dispatch::immediate(0)
    }

    /// `sys_ppu_thread_start`: no-op returning CELL_OK because
    /// [`Self::dispatch_ppu_thread_create`] schedules the new unit
    /// atomically; the thread is already running.
    ///
    /// # Known model gap (create/start ordering)
    ///
    /// Real LV2 creates threads SUSPENDED and transitions them to
    /// RUNNING here; CellGov collapses both into create. A title
    /// that observes the create-to-start interval would see a
    /// different ordering than real LV2 produces.
    pub(super) fn dispatch_ppu_thread_start(&self, _target: u64) -> Lv2Dispatch {
        Lv2Dispatch::immediate(0)
    }

    /// Returns `CELL_PPU_TIMEBASE_HZ` as the syscall code.
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
        // Match the on-entry snapshot every other inline arm uses.
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

    /// Wraps the inner `dispatch_ppu_thread_create` (in
    /// `host/ppu_thread.rs`) with an invariant-break log for any
    /// nonzero `flags` bits: `SYS_PPU_THREAD_CREATE_{JOINABLE,INTERRUPT}`
    /// are not modeled in the thread-table state, so the log
    /// surfaces a regression instead of dropping the field silently.
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
        // Valid LV2 priority range is 0..=3071; the downstream storage
        // is u32. A negative value reaches here only if the guest
        // passed garbage past the s!() sign-extension check, in which
        // case it casts to a large u32 and the scheduler ignores it
        // (the round-robin does not consult priority).
        let priority = priority as u32;
        self.dispatch_ppu_thread_create(id_ptr, param_ptr, arg, priority, stacksize, rt)
    }

    /// `sys_ss_access_control_engine`. Oracle:
    /// `rpcs3/Emu/Cell/lv2/sys_ss.cpp:157-201`. `pkg_id` 1/3 require
    /// debug-or-root and return ENOSYS for user-perm callers.
    /// `pkg_id == 2` writes the SELF program-authority-id
    /// (`PAID_44 = bdj.self`, from `rpcs3/Crypto/key_vault.h`) to
    /// `*a2`; matches cellSysmodule's recognised-caller branch in
    /// module_start. Any other `pkg_id` is SS-domain status
    /// `0x8001_051D`.
    pub(super) fn dispatch_ss_access_control_engine(
        &self,
        pkg_id: u64,
        a2: u64,
        requester: UnitId,
    ) -> Lv2Dispatch {
        match pkg_id {
            1 | 3 => Lv2Dispatch::immediate(errno::CELL_ENOSYS.into()),
            2 => match u32::try_from(a2) {
                Err(_) => Lv2Dispatch::immediate(errno::CELL_EFAULT.into()),
                Ok(0) => Lv2Dispatch::immediate(errno::CELL_EFAULT.into()),
                Ok(addr) => {
                    const PROGRAM_AUTHORITY_ID: u64 = 0x1070_0000_3A00_0001;
                    let authid_be = PROGRAM_AUTHORITY_ID.to_be_bytes();
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
    /// CELL_EINVAL rather than letting the call fall through to
    /// LV2. Guest-reachable -> `log_invariant_break`.
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
        Lv2Dispatch::immediate(errno::CELL_EINVAL.into())
    }

    /// `Unsupported` catch-all: logs once and returns
    /// CELL_ENOSYS.
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
        Lv2Dispatch::immediate(errno::CELL_ENOSYS.into())
    }

    /// `Malformed` rejection: classifier failed to bind the request
    /// fields. Guest-reachable; log + EINVAL.
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
        Lv2Dispatch::immediate(errno::CELL_EINVAL.into())
    }

    /// `UnresolvedImport` dispatch: the trampoline planted in an
    /// unpatched GOT slot fired. Log the NID + resolved name (if
    /// known) and return CELL_EINVAL so the title gets a real
    /// errno rather than control-flow corruption.
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
        Lv2Dispatch::immediate(errno::CELL_EINVAL.into())
    }
}
