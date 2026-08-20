//! Per-arm dispatch helpers for [`super::Lv2Host::dispatch`]'s typed
//! [`Lv2Request`] variants plus the `Unsupported` / `Malformed` /
//! `Hypercall` catch-alls.

use cellgov_effects::{Effect, WritePayload};
use cellgov_event::{PriorityClass, UnitId};
use cellgov_mem::ByteRange;
use cellgov_ps3_abi::cell_errors;

use crate::dispatch::Lv2Dispatch;

use crate::host::{Lv2Host, Lv2Runtime};
use cellgov_time::GuestTicks;

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

    pub(super) fn dispatch_memory_container_create(
        &mut self,
        cid_ptr: u32,
        requester: UnitId,
        tick: GuestTicks,
    ) -> Lv2Dispatch {
        let id = self.alloc_id();
        self.immediate_write_u32(id, cid_ptr, requester, tick)
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
        tick: GuestTicks,
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
            source_time: tick,
        };
        let dst_write = Effect::SharedWriteIntent {
            range: ByteRange::contiguous_u32(summer_time_ptr, 4),
            bytes: WritePayload::from_slice(&zero),
            ordering: PriorityClass::Normal,
            source: requester,
            source_time: tick,
        };
        Lv2Dispatch::Immediate {
            code: 0,
            effects: vec![tz_write, dst_write],
        }
    }

    /// Writes `(total, available)` to `*mem_info_ptr`; EFAULT on null.
    ///
    /// `total` is the PS3 game-mode user-memory cap. `available`
    /// subtracts what the bump allocator has handed out this boot
    /// (`sys_memory_free` is a no-op, so consumption is monotonic).
    /// The allocator's budget equals `total`, so the subtraction
    /// cannot underflow; `saturating_sub` guards the debug-only
    /// invariant anyway.
    ///
    /// Known divergence from the oracle (RPCS3 reports
    /// `container.size - container.used`): real LV2 charges the
    /// loaded image and every thread stack to the same container, so
    /// its first-read `available` is already below `total`. CellGov's
    /// counter starts at the post-image allocator base and thread
    /// stacks live in a separate region, so `available` over-reports
    /// by the image size plus stack usage.
    pub(super) fn dispatch_memory_get_user_memory_size(
        &self,
        mem_info_ptr: u32,
        requester: UnitId,
        tick: GuestTicks,
    ) -> Lv2Dispatch {
        if let Some(d) = self.efault_if_null(&[mem_info_ptr]) {
            return d;
        }
        let total = cellgov_ps3_abi::sys_memory::USER_MEMORY_TOTAL;
        // ptr starts at base and only grows; set_mem_alloc_base resets both.
        debug_assert!(self.state.mem_alloc_ptr >= self.derived.mem_alloc_base);
        let consumed = self.state.mem_alloc_ptr - self.derived.mem_alloc_base;
        let available = total.saturating_sub(consumed);
        let mut bytes = [0u8; 8];
        bytes[0..4].copy_from_slice(&total.to_be_bytes());
        bytes[4..8].copy_from_slice(&available.to_be_bytes());
        let write = Effect::SharedWriteIntent {
            range: ByteRange::contiguous_u32(mem_info_ptr, 8),
            bytes: WritePayload::from_slice(&bytes),
            ordering: PriorityClass::Normal,
            source: requester,
            source_time: tick,
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
        tick: GuestTicks,
    ) -> Lv2Dispatch {
        if let Some(d) = self.efault_if_null(&[sec_ptr, nsec_ptr]) {
            return d;
        }
        let (sec, nsec) = cellgov_time::ticks_to_sec_nsec(tick.raw());
        let sec_write = Effect::SharedWriteIntent {
            range: ByteRange::contiguous_u32(sec_ptr, 8),
            bytes: WritePayload::from_slice(&sec.to_be_bytes()),
            ordering: PriorityClass::Normal,
            source: requester,
            source_time: tick,
        };
        let nsec_write = Effect::SharedWriteIntent {
            range: ByteRange::contiguous_u32(nsec_ptr, 8),
            bytes: WritePayload::from_slice(&nsec.to_be_bytes()),
            ordering: PriorityClass::Normal,
            source: requester,
            source_time: tick,
        };
        Lv2Dispatch::Immediate {
            code: 0,
            effects: vec![sec_write, nsec_write],
        }
    }

    /// Wraps `dispatch_ppu_thread_create`: EPERM on JOINABLE+INTERRUPT
    /// together, a log on any other nonzero `flags` (single
    /// `SYS_PPU_THREAD_CREATE_{JOINABLE,INTERRUPT}` bits are not
    /// modeled). `threadname_ptr` is unconsumed: thread names have no
    /// modeled guest-visible surface.
    #[allow(clippy::too_many_arguments, reason = "mirrors the Lv2Request variant")]
    pub(super) fn dispatch_ppu_thread_create_with_flag_log(
        &mut self,
        id_ptr: u32,
        param_ptr: u32,
        arg: u64,
        unk: u64,
        priority: i32,
        stacksize: u64,
        flags: u64,
        threadname_ptr: u32,
        rt: &dyn Lv2Runtime,
    ) -> Lv2Dispatch {
        let _ = threadname_ptr;
        // The kernel ignores `unk` (RPCS3 `sys_ppu_thread.cpp`
        // `_sys_ppu_thread_create` only logs it; the sysPrxForUser
        // wrapper passes 0), so a nonzero value is decode evidence
        // worth keeping loud.
        if unk != 0 {
            self.log_invariant_break(
                "dispatch.ppu_thread_create_unconsumed_unk",
                format_args!("sys_ppu_thread_create unk=0x{unk:x} carries a nonzero value"),
            );
        }
        // JOINABLE and INTERRUPT together are refused (RPCS3
        // sys_ppu_thread.cpp _sys_ppu_thread_create returns CELL_EPERM
        // for (flags & 3) == 3). RPCS3 orders this check after the
        // entry EFAULT and priority EINVAL checks; those run inside
        // dispatch_ppu_thread_create here, so this refusal fires first
        // when both would apply.
        if flags & 3 == 3 {
            return Lv2Dispatch::immediate(cell_errors::CELL_EPERM.into());
        }
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
    /// user-perm callers. `pkg_id == 2` writes the CALLING process's
    /// program authority id to `*a2` (RPCS3 `sys_ss.cpp`
    /// `sys_ss_access_control_engine` serves the caller's per-process
    /// info) -- boot supplies its value from the title SELF's
    /// identification header via
    /// [`Lv2Host::set_program_authority_id`]; raw-ELF inputs and
    /// spawned children serve the retail-application fallback.
    /// Firmware modules classify callers by this value (libsysmodule's
    /// module_start skips its init entirely for recognized
    /// system-process ids), so it must name the caller's own SELF,
    /// never another process's. Any other `pkg_id` is SS-domain
    /// status `0x8001_051D`.
    pub(super) fn dispatch_ss_access_control_engine(
        &mut self,
        pkg_id: u64,
        a2: u64,
        requester: UnitId,
        tick: GuestTicks,
    ) -> Lv2Dispatch {
        match pkg_id {
            1 | 3 => Lv2Dispatch::immediate(cell_errors::CELL_ENOSYS.into()),
            2 => match u32::try_from(a2) {
                Err(_) => Lv2Dispatch::immediate(cell_errors::CELL_EFAULT.into()),
                Ok(0) => Lv2Dispatch::immediate(cell_errors::CELL_EFAULT.into()),
                Ok(addr) => {
                    let pid = self.state.processes.process_of_unit(requester);
                    let authority_id = match self.state.processes.get(pid) {
                        Some(entry) => entry.authority_id,
                        None => {
                            // Reachable only through a unit binding
                            // naming a pid the table never held; the
                            // boot value served here is a fabricated
                            // answer, so it never passes silently.
                            self.log_invariant_break(
                                "process.authority_of_unknown_pid",
                                format_args!(
                                    "access-control pkg 2 from {requester:?} bound to \
                                     pid {pid:#x} with no table entry; serving the \
                                     boot authority id"
                                ),
                            );
                            self.state.processes.boot().authority_id
                        }
                    };
                    let authid_be = authority_id.to_be_bytes();
                    let write = Effect::SharedWriteIntent {
                        range: ByteRange::contiguous_u32(addr, 8),
                        bytes: WritePayload::from_slice(&authid_be),
                        ordering: PriorityClass::Normal,
                        source: requester,
                        source_time: tick,
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
    pub(super) fn dispatch_timer_create(
        &mut self,
        id_ptr: u32,
        requester: UnitId,
        tick: GuestTicks,
    ) -> Lv2Dispatch {
        self.state.process_counts.timer_inc();
        let id = self.alloc_id();
        self.immediate_write_u32(id, id_ptr, requester, tick)
    }

    /// `sys_timer_destroy` stub: decrements the `ProcessCounts`
    /// timer counter and returns CELL_OK.
    pub(super) fn dispatch_timer_destroy(&mut self) -> Lv2Dispatch {
        self.state.process_counts.timer_dec();
        Lv2Dispatch::immediate(0)
    }

    /// `sys_rwlock_create` stub: mirrors [`Self::dispatch_timer_create`]
    /// against the rwlock counter.
    pub(super) fn dispatch_rwlock_create(
        &mut self,
        id_ptr: u32,
        requester: UnitId,
        tick: GuestTicks,
    ) -> Lv2Dispatch {
        self.state.process_counts.rwlock_inc();
        let id = self.alloc_id();
        self.immediate_write_u32(id, id_ptr, requester, tick)
    }

    /// `sys_rwlock_destroy` stub: mirrors [`Self::dispatch_timer_destroy`].
    pub(super) fn dispatch_rwlock_destroy(&mut self) -> Lv2Dispatch {
        self.state.process_counts.rwlock_dec();
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
        *self.obs.unsupported_syscalls.entry(number).or_insert(0) += 1;
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
    /// log NID + name (if in the db) + the library the import table
    /// asked for it from (if the boot recorded one), and return
    /// CELL_EINVAL.
    pub(super) fn dispatch_unresolved_import(
        &mut self,
        nid: u32,
        _requester: cellgov_event::UnitId,
    ) -> Lv2Dispatch {
        // The trampoline carries only the NID, so the library comes
        // from the requester map the GOT patcher installed, not from
        // the syscall. More than one library can appear when two
        // import tables both failed to resolve the same NID.
        // An absent entry and an empty set both mean "no recorded
        // library": an empty set must not leave a dangling
        // "imported from" with nothing after it.
        let requested_from = match self.obs.unresolved_import_requesters.get(&nid) {
            Some(libs) if !libs.is_empty() => {
                let list = libs.iter().map(String::as_str).collect::<Vec<_>>();
                format!(", imported from {}", list.join(", "))
            }
            _ => String::new(),
        };
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
                        "GOT slot for NID 0x{nid:08x} ({module_label}::{name}{requested_from}) \
                         was not bound by patch_got_atomic; returning CELL_EINVAL",
                    ),
                );
            }
            None => {
                self.log_invariant_break(
                    "dispatch.unresolved_import",
                    format_args!(
                        "GOT slot for NID 0x{nid:08x} (no name in NID db{requested_from}) was \
                         not bound by patch_got_atomic; returning CELL_EINVAL",
                    ),
                );
            }
        }
        Lv2Dispatch::immediate(cell_errors::CELL_EINVAL.into())
    }
}
