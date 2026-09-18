//! Per-arm dispatch helpers for [`super::Lv2Host::dispatch`]'s typed
//! [`Lv2Request`] variants plus the `Unsupported` / `Malformed` /
//! `Hypercall` catch-alls.

use cellgov_effects::{Effect, WritePayload};
use cellgov_event::UnitId;
use cellgov_mem::ByteRange;
use cellgov_ps3_abi::lv2::errno;

use crate::dispatch::Lv2Dispatch;

use crate::host::{Lv2Host, Lv2Runtime};
use cellgov_time::GuestTicks;

impl Lv2Host {
    /// `sys_spu_thread_group_terminate`: SPU teardown is not
    /// modeled; returns CELL_ENOSYS and logs an invariant break per
    /// call. What the kernel does to the group's running SPUs on
    /// terminate is unestablished; a console witness would fix it.
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

    /// `sys_memory_free`: the bump allocator tracks no per-allocation
    /// state, so a valid free, a bad pointer, and an unknown id are
    /// indistinguishable; all answer CELL_OK.
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

    /// `sys_ppu_thread_yield`: round-robin advance happens on the
    /// syscall itself, so the host returns CELL_OK with no effects.
    pub(super) fn dispatch_ppu_thread_yield(&self) -> Lv2Dispatch {
        Lv2Dispatch::immediate(0)
    }

    /// `sys_ppu_thread_start`: CELL_OK with no state change for a
    /// thread the table holds.
    ///
    /// Known gap: real LV2 creates threads SUSPENDED and transitions
    /// them here; CellGov collapses both into create, so the start
    /// itself has nothing to do.
    ///
    /// # Errors
    ///
    /// - `CELL_ESRCH` when `target` names no thread, as the join and
    ///   priority arms answer it.
    pub(super) fn dispatch_ppu_thread_start(&self, target: u64) -> Lv2Dispatch {
        if self
            .state
            .ppu_threads
            .get(crate::ppu_thread::PpuThreadId::new(target))
            .is_none()
        {
            return Lv2Dispatch::immediate(errno::CELL_ESRCH.into());
        }
        Lv2Dispatch::immediate(0)
    }

    /// `sys_time_get_timebase_frequency`: the fixed
    /// `CELL_PPU_TIMEBASE_HZ`, with no effects.
    pub(super) fn dispatch_time_get_timebase_frequency(&self) -> Lv2Dispatch {
        Lv2Dispatch::immediate(cellgov_time::CELL_PPU_TIMEBASE_HZ)
    }

    /// `sys_time_get_timezone`: writes zero through both out-pointers,
    /// UTC with no daylight saving; EFAULT on any null pointer. The
    /// arm reads no host clock.
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
        let tz_write = Effect::shared_write(
            ByteRange::contiguous_u32(timezone_ptr, 4),
            WritePayload::from_slice(&zero),
            requester,
            tick,
        );
        let dst_write = Effect::shared_write(
            ByteRange::contiguous_u32(summer_time_ptr, 4),
            WritePayload::from_slice(&zero),
            requester,
            tick,
        );
        Lv2Dispatch::Immediate {
            code: 0,
            effects: vec![tz_write, dst_write],
        }
    }

    /// Writes `(total, available)` to `*mem_info_ptr`; EFAULT on null.
    ///
    /// `total` is the PS3 game-mode user-memory cap; `available`
    /// subtracts what the bump allocator has handed out this boot
    /// (`sys_memory_free` is a no-op, so consumption is monotonic).
    ///
    /// `available` over-reports. Real LV2 charges the loaded image and
    /// every thread stack to the same container, so its first read is
    /// already below `total`. CellGov's counter starts at the
    /// post-image allocator base and holds thread stacks in a separate
    /// region, so the gap is the image size plus the stack usage.
    pub(super) fn dispatch_memory_get_user_memory_size(
        &self,
        mem_info_ptr: u32,
        requester: UnitId,
        tick: GuestTicks,
    ) -> Lv2Dispatch {
        if let Some(d) = self.efault_if_null(&[mem_info_ptr]) {
            return d;
        }
        let total = cellgov_ps3_abi::lv2::memory::USER_MEMORY_TOTAL;
        // ptr starts at base and only grows; set_mem_alloc_base resets both.
        debug_assert!(self.state.mem_alloc_ptr >= self.derived.mem_alloc_base);
        let consumed = self.state.mem_alloc_ptr - self.derived.mem_alloc_base;
        let available = total.saturating_sub(consumed);
        let mut bytes = [0u8; 8];
        bytes[0..4].copy_from_slice(&total.to_be_bytes());
        bytes[4..8].copy_from_slice(&available.to_be_bytes());
        let write = Effect::shared_write(
            ByteRange::contiguous_u32(mem_info_ptr, 8),
            WritePayload::from_slice(&bytes),
            requester,
            tick,
        );
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
        let sec_write = Effect::shared_write(
            ByteRange::contiguous_u32(sec_ptr, 8),
            WritePayload::from_slice(&sec.to_be_bytes()),
            requester,
            tick,
        );
        let nsec_write = Effect::shared_write(
            ByteRange::contiguous_u32(nsec_ptr, 8),
            WritePayload::from_slice(&nsec.to_be_bytes()),
            requester,
            tick,
        );
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
        // liblv2.sprx's `sys_ppu_thread_create` wrapper loads 0 into
        // the fourth syscall slot on every path, so no firmware caller
        // ever presents a value here. A nonzero `unk` is decode
        // evidence worth keeping loud.
        if unk != 0 {
            self.log_invariant_break(
                "dispatch.ppu_thread_create_unconsumed_unk",
                format_args!("sys_ppu_thread_create unk=0x{unk:x} carries a nonzero value"),
            );
        }
        // The interface defines `flags` as an OR over the JOINABLE and
        // INTERRUPT bits taken singly. A caller that sets both is
        // outside the defined set, and CellGov refuses it with
        // CELL_EPERM. Where that refusal sits against the entry
        // EFAULT and priority EINVAL gates is unestablished. Those
        // gates run inside dispatch_ppu_thread_create, so this one
        // fires first when more than one would apply.
        if flags & 3 == 3 {
            return Lv2Dispatch::immediate(errno::CELL_EPERM.into());
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

    /// `sys_ss_access_control_engine`.
    ///
    /// libsysmodule.sprx publishes one wrapper per `pkg_id` and issues
    /// only 1, 2 and 3. The 2 and 3 wrappers forward the caller's
    /// pointer in `a2` and load zero into `a3`, so `a2` is the out
    /// slot on those paths. A firmware classifier ranks the caller
    /// from the value the 2 wrapper yields.
    ///
    /// - `pkg_id` 1 and 3 require debug-or-root and return ENOSYS for
    ///   a user-perm caller.
    /// - `pkg_id` 2 writes the CALLING process's program authority id
    ///   to `*a2`. Boot supplies that value from the title SELF's
    ///   identification header via
    ///   [`Lv2Host::set_program_authority_id`]; raw-ELF inputs and
    ///   spawned children serve the retail-application fallback.
    ///   CELL_EFAULT when `a2` is zero or exceeds `u32`.
    /// - Any other `pkg_id` answers the SS-domain status
    ///   [`cellgov_ps3_abi::lv2::ss::SS_ACCESS_CONTROL_UNKNOWN_PKG_ID`].
    ///   All fourteen syscall-871 sites in the installed firmware --
    ///   eight modules, each site an immediate load -- put 1, 2 or 3
    ///   in `r3`. Nothing in that set reaches this arm, and the status
    ///   itself is unanchored.
    pub(super) fn dispatch_ss_access_control_engine(
        &mut self,
        pkg_id: u64,
        a2: u64,
        requester: UnitId,
        tick: GuestTicks,
    ) -> Lv2Dispatch {
        match pkg_id {
            1 | 3 => Lv2Dispatch::immediate(errno::CELL_ENOSYS.into()),
            2 => match u32::try_from(a2) {
                Err(_) => Lv2Dispatch::immediate(errno::CELL_EFAULT.into()),
                Ok(0) => Lv2Dispatch::immediate(errno::CELL_EFAULT.into()),
                Ok(addr) => {
                    let pid = self.state.processes.process_of_unit(requester);
                    let authority_id = match self.state.processes.get(pid) {
                        Some(entry) => entry.authority_id,
                        None => {
                            // Reachable only through a unit binding
                            // naming a pid the table never held.
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
                    let write = Effect::shared_write(
                        ByteRange::contiguous_u32(addr, 8),
                        WritePayload::from_slice(&authid_be),
                        requester,
                        tick,
                    );
                    Lv2Dispatch::Immediate {
                        code: 0,
                        effects: vec![write],
                    }
                }
            },
            _ => Lv2Dispatch::immediate(u64::from(
                cellgov_ps3_abi::lv2::ss::SS_ACCESS_CONTROL_UNKNOWN_PKG_ID,
            )),
        }
    }

    /// `sys_timer_create` stub: no timer state beyond the
    /// `ProcessCounts` tally is modeled.
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

    /// `sys_timer_destroy` stub: counterpart to
    /// [`Self::dispatch_timer_create`].
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

    /// PS3 usermode never issues `sc` with LEV != 0; CELL_EINVAL.
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

    /// `Unsupported` catch-all: CELL_ENOSYS.
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
        Lv2Dispatch::immediate(errno::CELL_ENOSYS.into())
    }

    /// `Malformed` rejection: the classifier could not bind the
    /// request's fields; CELL_EINVAL.
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

    /// `UnresolvedImport`: a trampoline in an unpatched GOT slot
    /// fired; CELL_EINVAL.
    pub(super) fn dispatch_unresolved_import(
        &mut self,
        nid: u32,
        _requester: cellgov_event::UnitId,
    ) -> Lv2Dispatch {
        // The trampoline carries only the NID, so the library comes
        // from the requester map the GOT patcher installed. More than
        // one library can appear when two import tables both failed to
        // resolve the same NID; an absent entry and an empty set both
        // mean "no recorded library".
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
        Lv2Dispatch::immediate(errno::CELL_EINVAL.into())
    }
}
