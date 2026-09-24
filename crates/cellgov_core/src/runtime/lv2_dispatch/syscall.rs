//! The syscall entry: classify a yield, dispatch the request, deliver the return, and drain invariant breaks.

use cellgov_event::UnitId;
use cellgov_exec::{ExecutionStepResult, UnitStatus};
use cellgov_lv2::{Lv2Dispatch, PendingResponse};
use cellgov_trace::{TraceRecord, TracedSyscallDisposition};

use crate::runtime::types::RuntimeMode;
use crate::runtime::{
    trace_bridge::{traced_invariant_break_reason, MemoryView},
    Runtime,
};

use super::checks::disposition_from_request;

impl Runtime {
    pub(in crate::runtime) fn dispatch_syscall(
        &mut self,
        result: &ExecutionStepResult,
        source: UnitId,
    ) {
        let Some(raw_args) = &result.syscall_args else {
            return;
        };
        // Synthetic / fake-ISA callers do not populate LEV.
        let lev = result.local_diagnostics.syscall_lev.unwrap_or(0);
        let num = raw_args[0];
        let args8: [u64; 8] = [
            raw_args[1],
            raw_args[2],
            raw_args[3],
            raw_args[4],
            raw_args[5],
            raw_args[6],
            raw_args[7],
            raw_args[8],
        ];

        use cellgov_ps3_abi::lv2::syscall::{TIMER_SLEEP, TIMER_USLEEP};
        let is_timer_fast_path = lev == 0 && cellgov_lv2::request::RUNTIME_FAST_PATH.contains(&num);

        // Classify upfront so the entry record can carry the
        // disposition byte. Timer fast-path skips classify (the
        // disposition is known by shape and the path bypasses
        // `Lv2Host::dispatch` entirely).
        let (disposition, request) = if is_timer_fast_path {
            (TracedSyscallDisposition::TimerFastPath, None)
        } else {
            let req = cellgov_lv2::request::classify_with_lev(lev, num, &args8);
            let d = if lev != 0 {
                TracedSyscallDisposition::Hypercall
            } else {
                disposition_from_request(&req)
            };
            (d, Some(req))
        };

        // Emit the entry record before any state mutation.
        if self.mode != RuntimeMode::FaultDriven {
            self.trace.record(&TraceRecord::SyscallEntered {
                unit: source,
                num,
                args: args8,
                disposition,
            });
        }

        if let Some(request) = request {
            self.dispatch_lv2_request_with_ordinal(request, source, (lev == 0).then_some(num));
            return;
        }

        // Timer path: park the caller until guest time reaches the
        // requested interval, still bypassing `Lv2Host::dispatch`.
        // Other threads run meanwhile; when everything is parked, the
        // all-blocked time-warp jumps the clock to the deadline.
        // Nothing public attests that the caller parks and then
        // returns CELL_OK; a console probe or an lv2 timer autotest
        // would settle it.
        let usec = match num {
            TIMER_USLEEP => args8[0],
            // The seconds-granularity call takes an unsigned 32-bit
            // second count, so only the low word of the argument
            // register carries a value. u32::MAX seconds fits in u64
            // microseconds, so the scale cannot overflow.
            TIMER_SLEEP => u64::from(args8[0] as u32) * 1_000_000,
            _ => unreachable!("is_timer_fast_path implies num is TIMER_USLEEP or TIMER_SLEEP"),
        };
        self.timer_sleep_dispatches = self.timer_sleep_dispatches.saturating_add(1);
        if usec == 0 {
            // Zero-interval sleep is a yield, not a park.
            self.deliver_syscall_return(source, 0);
            return;
        }
        let deadline = self.deadline_after_usec(usec);
        let displaced = self
            .syscall_responses
            .insert(source, PendingResponse::ReturnCode { code: 0 });
        if let Some(prev) = &displaced {
            self.lv2_host.log_invariant_break(
                "runtime.dispatch_syscall_timer_park_pending_response_displaced",
                format_args!(
                    "{source:?} parked on a timer sleep with {prev:?} still pending, so the \
                     earlier response is overwritten and its wake never reaches the guest"
                ),
            );
        }
        debug_assert!(
            displaced.is_none(),
            "timer park: source {source:?} already had a pending response: {displaced:?}"
        );
        let displaced_wake =
            self.timer_wakes
                .insert(deadline, source, crate::timer_queue::TimerWakeKind::Sleep);
        if let Some(prior) = displaced_wake {
            self.lv2_host.log_invariant_break(
                "runtime.dispatch_syscall_timer_park_timer_wake_displaced",
                format_args!(
                    "{source:?} parked on a timer sleep with {prior:?} still live, so the wake \
                     path that resolved its previous wait failed to cancel the deadline"
                ),
            );
        }
        self.registry
            .set_status_override(source, UnitStatus::Blocked);
    }

    /// Store a syscall's return value for `unit`'s next step and trace
    /// it. Every path that resolves a syscall -- immediate, wake, timer
    /// -- delivers through here, so the stream carries one
    /// `SyscallReturned` per value the guest sees in `r3`.
    pub(in crate::runtime) fn deliver_syscall_return(&mut self, unit: UnitId, code: u64) {
        if self.mode != RuntimeMode::FaultDriven {
            self.trace.record(&TraceRecord::SyscallReturned {
                unit,
                code,
                time: self.time,
            });
        }
        self.registry.set_syscall_return(unit, code);
    }

    /// Drain buffered LV2 invariant breaks at the boundary that
    /// produced them. Always drains so the buffer stays bounded; only
    /// emits trace records under modes that write a trace stream
    /// (FaultDriven consults `invariant_break_count` via the boot
    /// summary). Called after every host dispatch and after timer-wake
    /// firing, so a break logged during expiry is not attributed to
    /// the next syscall's dispatch boundary.
    pub(in crate::runtime) fn drain_invariant_breaks_to_trace(&mut self) {
        if self.mode == RuntimeMode::FaultDriven {
            for _ in self.lv2_host.drain_pending_invariant_breaks() {}
        } else {
            let reasons: Vec<_> = self.lv2_host.drain_pending_invariant_breaks().collect();
            for reason in reasons {
                self.trace.record(&TraceRecord::HostInvariantBreak {
                    reason: traced_invariant_break_reason(reason),
                });
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn dispatch_lv2_request(
        &mut self,
        request: cellgov_lv2::Lv2Request,
        source: UnitId,
    ) {
        self.dispatch_lv2_request_inner(request, source, None);
    }

    fn dispatch_lv2_request_with_ordinal(
        &mut self,
        request: cellgov_lv2::Lv2Request,
        source: UnitId,
        ordinal: Option<u64>,
    ) {
        self.dispatch_lv2_request_inner(request, source, ordinal);
    }

    fn dispatch_lv2_request_inner(
        &mut self,
        request: cellgov_lv2::Lv2Request,
        source: UnitId,
        ordinal: Option<u64>,
    ) {
        // Both exit forms terminate the calling process, and an
        // exit-and-spawn whose argv walk finds no target is just an
        // exit. A ProcessExit2 that resolves to Immediate(0) must
        // therefore trigger the same finish-all sweep as a plain exit,
        // or the guest resumes past a noreturn call. Child
        // exits arrive as Lv2Dispatch::ProcessExitChild and never
        // reach the Immediate arm, so the flag stays boot-only there.
        let is_process_exit = matches!(
            request,
            cellgov_lv2::Lv2Request::ProcessExit { .. }
                | cellgov_lv2::Lv2Request::ProcessExit2 { .. }
        );
        let wait_timeout_usec = request.wait_timeout_usec();
        let memory = MemoryView {
            memory: crate::runtime::spaces::resolve_unit_memory(&self.memory, &self.spaces, source),
            current_tick: self.time,
        };
        let dispatch = match ordinal {
            Some(ordinal) => self
                .lv2_host
                .dispatch_with_ordinal(request, source, &memory, ordinal),
            None => self.lv2_host.dispatch(request, source, &memory),
        };
        self.drain_invariant_breaks_to_trace();
        // Apply shm region-install requests before the dispatch's effects
        // commit: a 334 that mints a fresh region and an effect targeting
        // that region in the same dispatch would otherwise hit
        // `CommitError::OutOfRange` at pre-validation. Collect into a
        // local so the borrow on `self.lv2_host` releases before
        // touching `self.memory`.
        let region_installs: Vec<(u64, usize, Option<u64>)> =
            self.lv2_host.drain_pending_region_installs().collect();
        if !region_installs.is_empty() {
            // The mapping appears in the CALLER's address space. The
            // syscall takes no process argument, so the caller's space
            // is the only one it can name. The handler validated the
            // window against the caller's view, and the caller's own
            // loads/stores resolve through that space.
            let caller_space = self.spaces.space_of(source);
            for (addr, size, ipc_key) in region_installs {
                let mem = crate::runtime::spaces::resolve_space_memory_for_write(
                    &mut self.memory,
                    &mut self.spaces,
                    caller_space,
                );
                if let Err(err) =
                    mem.install_region(addr, size, "shm", cellgov_mem::PageSize::Page64K)
                {
                    // Guest-reachable, but no longer expected: sc 334
                    // now refuses an occupied window with CELL_EBUSY
                    // up front, consulting both the host ledger and
                    // the caller's committed layout, and sc 337's
                    // search skips both. Reaching this arm means a
                    // window passed those gates and still failed to
                    // install, so the break names a gap between the
                    // gates and the caller's real layout. The window
                    // stays unmapped, so the caller's next access to
                    // it faults instead of aliasing.
                    self.lv2_host.log_invariant_break(
                        "dispatch.mmapper_region_install_overlap",
                        format_args!(
                            "shm region install at 0x{addr:08x}+0x{size:x} overlaps the \
                             caller's existing layout in space {}: {err}; window not \
                             installed, the guest already observed success where the \
                             kernel would return CELL_EBUSY",
                            caller_space.raw(),
                        ),
                    );
                    continue;
                }
                // A keyed window is a view of a process-shared
                // segment; record it so views stay coherent across
                // address spaces once a second space attaches.
                if let Some(key) = ipc_key {
                    self.attach_keyed_shm_view(key, size as u64, caller_space, addr);
                }
            }
        }
        match dispatch {
            Lv2Dispatch::Immediate { code, effects } => {
                if is_process_exit {
                    self.step_woke_others = true;
                }
                self.handle_immediate(source, code, effects, is_process_exit);
            }
            Lv2Dispatch::ImmediateRegisters {
                code,
                effects,
                registers,
            } => {
                self.handle_immediate(source, code, effects, false);
                for (reg, value) in registers {
                    self.registry.push_register_write(source, reg, value);
                }
            }
            Lv2Dispatch::RegisterSpu {
                inits,
                effects,
                code,
            } => {
                self.handle_register_spu(source, inits, effects, code);
            }
            Lv2Dispatch::Block {
                reason,
                pending,
                effects,
            } => {
                self.handle_block(source, pending, effects);
                self.register_wait_deadline(source, reason, wait_timeout_usec);
            }
            Lv2Dispatch::PpuThreadExit {
                exit_value,
                woken_unit_ids,
                lwmutex_inheritors,
                effects,
            } => {
                if !woken_unit_ids.is_empty() || !lwmutex_inheritors.is_empty() {
                    self.step_woke_others = true;
                }
                self.handle_ppu_thread_exit(
                    source,
                    exit_value,
                    woken_unit_ids,
                    lwmutex_inheritors,
                    effects,
                );
            }
            Lv2Dispatch::PpuThreadCreate { .. } => {
                self.step_woke_others = true;
                self.handle_ppu_thread_create(source, dispatch);
            }
            Lv2Dispatch::ProcessSpawn { .. } => {
                self.handle_process_spawn(source, dispatch);
            }
            Lv2Dispatch::ProcessExitChild { pid, code, effects } => {
                self.handle_process_exit_child(source, pid, code, effects);
            }
            Lv2Dispatch::WakeAndReturn {
                code,
                woken_unit_ids,
                response_updates,
                effects,
            } => {
                if !woken_unit_ids.is_empty() {
                    self.step_woke_others = true;
                }
                self.handle_wake_and_return(
                    source,
                    code,
                    woken_unit_ids,
                    response_updates,
                    effects,
                );
            }
            Lv2Dispatch::BlockAndWake {
                reason,
                pending,
                woken_unit_ids,
                response_updates,
                effects,
            } => {
                if !woken_unit_ids.is_empty() {
                    self.step_woke_others = true;
                }
                self.handle_block_and_wake(
                    source,
                    pending,
                    woken_unit_ids,
                    response_updates,
                    effects,
                );
                self.register_wait_deadline(source, reason, wait_timeout_usec);
            }
        }
    }
}
