//! Bridges `Lv2Request`/`Lv2Dispatch` to `Runtime` mutation: classify a
//! syscall yield, route it through `Lv2Host::dispatch`, and fold the
//! result back into syscall responses, registry status, and mailbox
//! state. `handle_ppu_thread_create` lives in `ppu_create.rs`.

use std::collections::BTreeMap;

use cellgov_effects::Effect;
use cellgov_event::UnitId;
use cellgov_exec::{ExecutionStepResult, UnitStatus};
use cellgov_lv2::{Lv2Dispatch, PendingResponse, SpuInitState};
use cellgov_mem::MemError;
use cellgov_trace::{TraceRecord, TracedSyscallDisposition};

use super::spaces::AddressSpaceId;
use super::types::RuntimeMode;
use super::{
    trace_bridge::{traced_invariant_break_reason, MemoryView},
    Runtime,
};

impl Runtime {
    /// Apply an `Lv2Dispatch` effects batch by direct commit,
    /// bypassing the commit pipeline's [`StagingMemory`].
    ///
    /// The `SharedWriteIntent` subset commits all-or-none: a
    /// validation failure logs a `dispatch.lv2_effect_apply_failed`
    /// invariant break and lands none of the writes, while non-memory
    /// effects still apply. A requested flip transitions
    /// WAITING -> DONE on the next `commit_step` boundary, not during
    /// the dispatching batch. Variants with no LV2 producer (e.g.
    /// `RsxLabelWrite`, `DmaEnqueue`) are dropped with a named
    /// invariant break; they belong to the unit-effect path through
    /// `commit_pipeline.process`.
    ///
    /// [`StagingMemory`]: cellgov_mem::StagingMemory
    ///
    /// # Atomic-batch discard semantics
    ///
    /// These mutations DO NOT participate in atomic-batch
    /// discard-on-fault: by the time the containing batch finalizes,
    /// the syscall has already returned its result to the guest, so
    /// syscall-side state persists even when the batch's unit-staged
    /// effects are discarded.
    ///
    /// # Ordering against unit `SharedWriteIntent`s
    ///
    /// `Runtime::commit_step` drains staged unit effects first and
    /// calls this function second, so an LV2 write deterministically
    /// lands after any same-batch unit store to the same range.
    ///
    /// # Cross-module contract
    ///
    /// LV2 handlers must not emit a non-memory effect whose
    /// semantics depend on a co-batched `SharedWriteIntent` having
    /// committed; a `debug_assert!` below traps that condition.
    ///
    /// `space` is the address space the memory subset commits into:
    /// the syscall CALLER's space for dispatch-time effects, the
    /// expiring waiter's space for `expire_wait` effects. LV2 handlers
    /// must emit only intents whose pointers were decoded from that
    /// unit's own syscall, so every intent is an address in `space`
    /// (a handler emitting another unit's parked pointer as an effect
    /// would commit into the wrong space; waiter-side payloads belong
    /// on `PendingResponse` / `response_updates` instead).
    pub(super) fn apply_lv2_effects(&mut self, effects: &[Effect], space: AddressSpaceId) {
        let memory_failure = self.validate_lv2_memory_subset(effects, space);
        debug_assert!(
            !(memory_failure.is_some()
                && effects
                    .iter()
                    .any(|e| !matches!(e, Effect::SharedWriteIntent { .. }))),
            "LV2 handler co-emitted a SharedWriteIntent that failed validation alongside \
             a non-memory effect; non-memory effects land unconditionally, leaving the \
             batch in a forbidden partial state. failure={:?}",
            memory_failure,
        );
        if let Some((range, err)) = memory_failure {
            let addr = range.start().raw();
            let length = range.length();
            self.lv2_host.log_invariant_break(
                "dispatch.lv2_effect_apply_failed",
                format_args!(
                    "LV2 SharedWriteIntent validation failed at addr=0x{addr:016x} \
                     length={length} in space {}: {err}; memory subset rolled back, \
                     non-memory effects still applied",
                    space.raw(),
                ),
            );
        }
        for effect in effects {
            match effect {
                Effect::SharedWriteIntent { range, bytes, .. } => {
                    if memory_failure.is_some() {
                        continue;
                    }
                    // LV2 direct commits bypass the commit pipeline's
                    // shared-view fanout; a write landing inside a
                    // shared view would leave sibling views incoherent.
                    // No LV2 emitter targets shared segments yet --
                    // surface the first one here instead of as silent
                    // incoherence (same guard as ConditionalStore in
                    // fanout_shared_writes). The invariant break keeps
                    // the witness loud in release builds, where the
                    // debug_assert compiles out and the write would
                    // otherwise land in this view alone.
                    if self.range_intersects_shared_view(space, *range) {
                        self.lv2_host.log_invariant_break(
                            "dispatch.lv2_write_targets_shared_view",
                            format_args!(
                                "LV2 SharedWriteIntent at 0x{:x}+0x{:x} targets a shared \
                                 view in space {}; cross-space replication of LV2 direct \
                                 commits is not modeled, sibling views are now incoherent",
                                range.start().raw(),
                                range.length(),
                                space.raw(),
                            ),
                        );
                        debug_assert!(
                            false,
                            "LV2 SharedWriteIntent at {:#x}+{:#x} targets a shared view in \
                             space {}; cross-space replication of LV2 direct commits is not \
                             modeled",
                            range.start().raw(),
                            range.length(),
                            space.raw(),
                        );
                    }
                    let mem = super::spaces::resolve_space_memory_for_write(
                        &mut self.memory,
                        &mut self.spaces,
                        space,
                    );
                    mem.apply_commit(*range, bytes.bytes()).expect(
                        "validate_lv2_memory_subset called GuestMemory::validate_write -- \
                         the same predicate apply_commit uses internally -- so this Err \
                         path is structurally unreachable",
                    );
                    self.lv2_direct_committed_writes =
                        self.lv2_direct_committed_writes.wrapping_add(1);
                }
                Effect::MailboxSend {
                    mailbox, message, ..
                } => {
                    if let Some(mbox) = self.mailbox_registry.get_mut(*mailbox) {
                        // [CBE-Handbook p:541 s:19.6.6.2] outbound
                        // write-blocking path is not wired here yet.
                        mbox.force_send(message.raw());
                    }
                    let target = UnitId::new(mailbox.raw());
                    if self.registry.effective_status(target) == Some(UnitStatus::Blocked) {
                        self.registry
                            .set_status_override(target, UnitStatus::Runnable);
                    }
                }
                Effect::RsxFlipRequest { buffer_index } => {
                    self.rsx_flip.request_flip(*buffer_index);
                }
                Effect::MailboxReceiveAttempt { .. } => {
                    self.lv2_host.log_invariant_break(
                        "runtime.apply_lv2_effects_unsupported_mailbox_receive_attempt",
                        format_args!(
                            "LV2 dispatch emitted MailboxReceiveAttempt; this variant is \
                             PPU/SPU-side receiver semantics and has no LV2 producer. \
                             Effect dropped."
                        ),
                    );
                }
                Effect::DmaEnqueue { .. } => {
                    self.lv2_host.log_invariant_break(
                        "runtime.apply_lv2_effects_unsupported_dma_enqueue",
                        format_args!(
                            "LV2 dispatch emitted DmaEnqueue; this variant originates \
                             from SPU MFC issue. Effect dropped."
                        ),
                    );
                }
                Effect::WaitOnEvent { .. } => {
                    self.lv2_host.log_invariant_break(
                        "runtime.apply_lv2_effects_unsupported_wait_on_event",
                        format_args!(
                            "LV2 dispatch emitted WaitOnEvent; LV2 block semantics use \
                             Lv2Dispatch::Block / PendingResponse, not this variant. \
                             Effect dropped."
                        ),
                    );
                }
                Effect::WakeUnit { .. } => {
                    self.lv2_host.log_invariant_break(
                        "runtime.apply_lv2_effects_unsupported_wake_unit",
                        format_args!(
                            "LV2 dispatch emitted WakeUnit; LV2 wake semantics use the \
                             `woken_unit_ids` field on Lv2Dispatch::WakeAndReturn, not \
                             this variant. Effect dropped."
                        ),
                    );
                }
                Effect::SignalUpdate { .. } => {
                    self.lv2_host.log_invariant_break(
                        "runtime.apply_lv2_effects_unsupported_signal_update",
                        format_args!(
                            "LV2 dispatch emitted SignalUpdate; this variant originates \
                             from PPU/SPU signal-write paths. Effect dropped."
                        ),
                    );
                }
                Effect::FaultRaised { .. } => {
                    self.lv2_host.log_invariant_break(
                        "runtime.apply_lv2_effects_unsupported_fault_raised",
                        format_args!(
                            "LV2 dispatch emitted FaultRaised; faults originate from \
                             execution units, not LV2 syscall handlers. Effect dropped."
                        ),
                    );
                }
                Effect::TraceMarker { .. } => {
                    self.lv2_host.log_invariant_break(
                        "runtime.apply_lv2_effects_unsupported_trace_marker",
                        format_args!(
                            "LV2 dispatch emitted TraceMarker; this is an execution-unit \
                             breadcrumb variant. Effect dropped."
                        ),
                    );
                }
                Effect::ReservationAcquire { .. } => {
                    self.lv2_host.log_invariant_break(
                        "runtime.apply_lv2_effects_unsupported_reservation_acquire",
                        format_args!(
                            "LV2 dispatch emitted ReservationAcquire; LL/SC paths run in \
                             execution units, not LV2 syscalls. Effect dropped."
                        ),
                    );
                }
                Effect::ConditionalStore { .. } => {
                    self.lv2_host.log_invariant_break(
                        "runtime.apply_lv2_effects_unsupported_conditional_store",
                        format_args!(
                            "LV2 dispatch emitted ConditionalStore; stwcx./stdcx. paths \
                             run in execution units, not LV2 syscalls. Effect dropped."
                        ),
                    );
                }
                Effect::RsxLabelWrite { .. } => {
                    self.lv2_host.log_invariant_break(
                        "runtime.apply_lv2_effects_unsupported_rsx_label_write",
                        format_args!(
                            "LV2 dispatch emitted RsxLabelWrite; this variant flows from \
                             the FIFO advance pass via pending_rsx_effects into the next \
                             batch's commit_pipeline, not through LV2 dispatch. Effect dropped."
                        ),
                    );
                }
            }
        }
    }

    /// Pre-validate the `SharedWriteIntent` subset of `effects`
    /// against `space`'s memory via `GuestMemory::validate_write`
    /// (the predicate `apply_commit` itself calls). Returns the first
    /// failing intent or `None`.
    fn validate_lv2_memory_subset(
        &self,
        effects: &[Effect],
        space: AddressSpaceId,
    ) -> Option<(cellgov_mem::ByteRange, MemError)> {
        let mem = super::spaces::resolve_space_memory(&self.memory, &self.spaces, space);
        for effect in effects {
            if let Effect::SharedWriteIntent { range, bytes, .. } = effect {
                if let Err(err) = mem.validate_write(*range, bytes.len()) {
                    return Some((*range, err));
                }
            }
        }
        None
    }

    pub(super) fn dispatch_syscall(&mut self, result: &ExecutionStepResult, source: UnitId) {
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

        use cellgov_ps3_abi::syscall::{TIMER_SLEEP, TIMER_USLEEP};
        let is_timer_fast_path = lev == 0 && (num == TIMER_USLEEP || num == TIMER_SLEEP);

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
            self.dispatch_lv2_request(request, source);
            return;
        }

        // Timer path: park the caller until guest time reaches the
        // requested interval, still bypassing `Lv2Host::dispatch`.
        // Other threads run meanwhile; when everything is parked, the
        // all-blocked time-warp jumps the clock to the deadline (RPCS3
        // rpcs3/Emu/Cell/lv2/sys_timer.cpp sys_timer_usleep /
        // sys_timer_sleep: the thread enters a timed sleep and always
        // returns CELL_OK).
        let usec = match num {
            TIMER_USLEEP => args8[0],
            // The seconds argument is a 32-bit value; the kernel reads
            // only the low word of the register (RPCS3 sys_timer.cpp
            // sys_timer_sleep takes u32 sleep_time). u32::MAX seconds
            // fits in u64 microseconds, so the scale cannot overflow.
            TIMER_SLEEP => u64::from(args8[0] as u32) * 1_000_000,
            _ => unreachable!("is_timer_fast_path implies num is TIMER_USLEEP or TIMER_SLEEP"),
        };
        self.timer_sleep_dispatches = self.timer_sleep_dispatches.saturating_add(1);
        if usec == 0 {
            // Zero-interval sleep is a yield, not a park.
            self.registry.set_syscall_return(source, 0);
            return;
        }
        let deadline = self.deadline_after_usec(usec);
        let displaced = self
            .syscall_responses
            .insert(source, PendingResponse::ReturnCode { code: 0 });
        debug_assert!(
            displaced.is_none(),
            "timer park: source {source:?} already had a pending response: {displaced:?}"
        );
        self.timer_wakes
            .insert(deadline, source, crate::timer_queue::TimerWakeKind::Sleep);
        self.registry
            .set_status_override(source, UnitStatus::Blocked);
    }

    /// Drain buffered LV2 invariant breaks at the boundary that
    /// produced them. Always drains so the buffer stays bounded; only
    /// emits trace records under modes that write a trace stream
    /// (FaultDriven consults `invariant_break_count` via the boot
    /// summary). Called after every host dispatch and after timer-wake
    /// firing, so a break logged during expiry is not attributed to
    /// the next syscall's dispatch boundary.
    pub(super) fn drain_invariant_breaks_to_trace(&mut self) {
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

    pub(crate) fn dispatch_lv2_request(
        &mut self,
        request: cellgov_lv2::Lv2Request,
        source: UnitId,
    ) {
        // ProcessExit2 with empty argv resolves to the same plain
        // process exit (RPCS3 sys_process.cpp _sys_process_exit2
        // calls _sys_process_exit when the argv walk finds nothing),
        // so its Immediate(0) must trigger the same finish-all sweep
        // -- otherwise the guest resumes past a noreturn exit. Child
        // exits arrive as Lv2Dispatch::ProcessExitChild and never
        // reach the Immediate arm, so the flag stays boot-only there.
        let is_process_exit = matches!(
            request,
            cellgov_lv2::Lv2Request::ProcessExit { .. }
                | cellgov_lv2::Lv2Request::ProcessExit2 { .. }
        );
        let wait_timeout_usec = request.wait_timeout_usec();
        let dispatch = self.lv2_host.dispatch(
            request,
            source,
            // Syscall parameters are read from the CALLER's space.
            &MemoryView {
                memory: crate::runtime::spaces::resolve_unit_memory(
                    &self.memory,
                    &self.spaces,
                    source,
                ),
                current_tick: self.time,
            },
        );
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
            // The mapping appears in the CALLER's address space: the
            // handler validated the window against the caller's view,
            // and the caller's own loads/stores resolve through its
            // space (RPCS3 sys_mmapper.cpp sys_mmapper_map_shared_memory
            // maps into the calling process's virtual memory).
            let caller_space = self.spaces.space_of(source);
            for (addr, size, ipc_key) in region_installs {
                let mem = super::spaces::resolve_space_memory_for_write(
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
            Lv2Dispatch::RegisterSpu {
                inits,
                effects,
                code,
            } => {
                if !inits.is_empty() {
                    self.step_woke_others = true;
                }
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

    /// `ProcessExit` finishes every unit and drops parked responses.
    fn handle_immediate(
        &mut self,
        source: UnitId,
        code: u64,
        effects: Vec<Effect>,
        is_process_exit: bool,
    ) {
        let caller_space = self.spaces.space_of(source);
        self.apply_lv2_effects(&effects, caller_space);
        if is_process_exit {
            let all_ids: Vec<UnitId> = self.registry.ids().collect();
            for uid in &all_ids {
                self.registry
                    .set_status_override(*uid, UnitStatus::Finished);
                // UnknownUnit (non-SPU) and AlreadyFinished are
                // expected during the per-unit sweep.
                match self.lv2_host.notify_spu_finished(*uid) {
                    Ok(_)
                    | Err(cellgov_lv2::thread_group::NotifySpuFinishedError::UnknownUnit)
                    | Err(cellgov_lv2::thread_group::NotifySpuFinishedError::AlreadyFinished {
                        ..
                    }) => {}
                    Err(err) => {
                        #[allow(
                            clippy::print_stderr,
                            reason = "diagnostic for an LV2 host invariant break reachable only when thread-table state diverges from primitive state; one line per offending unit per host instance"
                        )]
                        {
                            eprintln!(
                                "lv2 host invariant break at process_exit.notify_spu_finished: \
                                 unit {:?}: {err:?}",
                                uid,
                            );
                        }
                    }
                }
                // The parked response is dropped, so the unit's timer
                // entry must go with it: fire_timer_wakes runs later
                // in this same commit, and a deadline the exiting
                // step just crossed would otherwise wake a unit this
                // sweep finished (dropping a response without
                // cancelling its deadline is the invariant break
                // `Lv2Host::expire_wait` names).
                self.timer_wakes.cancel(*uid);
                let _ = self.syscall_responses.try_take(*uid);
            }
        } else {
            self.registry.set_syscall_return(source, code);
        }
    }

    /// `BTreeMap` iteration keeps registration order byte-stable.
    fn handle_register_spu(
        &mut self,
        source: UnitId,
        inits: BTreeMap<u32, SpuInitState>,
        effects: Vec<Effect>,
        code: u64,
    ) {
        let caller_space = self.spaces.space_of(source);
        self.apply_lv2_effects(&effects, caller_space);
        if let Some(factory) = &self.spu_factory {
            for (slot, init) in inits {
                let gid = init.group_id;
                let uid = self
                    .registry
                    .register_dynamic(&|id| factory(id, init.clone()));
                self.lv2_host.record_spu(uid, gid, slot).expect(
                    "record_spu rejected a freshly allocated unit: dispatch-layer \
                     corruption in the RegisterSpu path",
                );
                // SPU Read Inbound Mailbox depth is 4 per
                // [CBE-Handbook p:533 s:19.6 Table 19-15]; we use it
                // as the default capacity for dispatch-allocated
                // mailboxes until the SPU exec layer differentiates
                // outbound (depth 1) from inbound (depth 4).
                const SPU_INBOUND_MBOX_DEPTH: usize = 4;
                let inserted = self.mailbox_registry.register_at(
                    cellgov_sync::MailboxId::new(uid.raw()),
                    SPU_INBOUND_MBOX_DEPTH,
                );
                if !inserted {
                    // Collision means the dispatch layer reused a
                    // UnitId that already had a mailbox -- SPU
                    // mailbox state would silently cross-talk
                    // between units.
                    #[allow(
                        clippy::print_stderr,
                        reason = "one-shot release-build diagnostic for SPU mailbox id collision; not guest-reachable under normal operation"
                    )]
                    {
                        eprintln!(
                            "lv2 host invariant break at dispatch.register_spu_mailbox_collision: \
                             UnitId({:?}) reused an existing mailbox slot; SPU mailbox crosstalk \
                             is possible. Baseline anchors must be re-validated if this fires.",
                            uid.raw()
                        );
                    }
                    debug_assert!(
                        inserted,
                        "RegisterSpu for UnitId({:?}) found an existing mailbox; \
                         the dispatch layer must allocate a fresh unit id per SPU",
                        uid.raw()
                    );
                }
            }
        }
        self.registry.set_syscall_return(source, code);
    }

    fn handle_block(&mut self, source: UnitId, pending: PendingResponse, effects: Vec<Effect>) {
        let caller_space = self.spaces.space_of(source);
        self.apply_lv2_effects(&effects, caller_space);
        let displaced = self.syscall_responses.insert(source, pending);
        if let Some(prev) = &displaced {
            // Displacement overwrites a pending response, losing the
            // original wake. SyscallResponseTable::insert log-once
            // covers first occurrence; this site adds source/variant
            // for cross-syscall attribution.
            #[allow(
                clippy::print_stderr,
                reason = "one-shot release-build diagnostic for pending-response displacement; not guest-reachable under normal operation"
            )]
            {
                eprintln!(
                    "lv2 host invariant break at dispatch.handle_block.displacement: \
                     source {source:?} already had pending response {prev:?}; \
                     new response will be silently overwritten."
                );
            }
        }
        debug_assert!(
            displaced.is_none(),
            "handle_block: source {source:?} already had a pending response: {displaced:?}"
        );
        self.registry
            .set_status_override(source, UnitStatus::Blocked);
    }

    /// Waiters without a matching `PpuThreadJoin` response wake with
    /// the raw exit value in r3 instead of writing through the out
    /// pointer.
    fn handle_ppu_thread_exit(
        &mut self,
        source: UnitId,
        exit_value: u64,
        woken_unit_ids: Vec<UnitId>,
        lwmutex_inheritors: Vec<UnitId>,
        effects: Vec<Effect>,
    ) {
        let caller_space = self.spaces.space_of(source);
        self.apply_lv2_effects(&effects, caller_space);
        self.registry
            .set_status_override(source, UnitStatus::Finished);
        for waiter in woken_unit_ids {
            self.timer_wakes.cancel(waiter);
            let pending = self.syscall_responses.try_take(waiter);
            if let Some(PendingResponse::PpuThreadJoin { status_out_ptr, .. }) = pending {
                // The out-pointer was decoded from the JOINER's
                // syscall, so it addresses the joiner's space.
                if status_out_ptr != 0 {
                    let waiter_space = self.spaces.space_of(waiter);
                    self.commit_bytes_at(
                        waiter_space,
                        status_out_ptr as u64,
                        &exit_value.to_be_bytes(),
                    );
                    self.registry.set_syscall_return(waiter, 0);
                } else {
                    // A NULL out-pointer is never written -- the join
                    // itself still completes (the target is reaped),
                    // but the joiner's r3 reports CELL_EFAULT, not
                    // success (RPCS3 sys_ppu_thread.cpp
                    // sys_ppu_thread_join checks vptr only after the
                    // wait resolves and returns CELL_EFAULT for null).
                    self.registry.set_syscall_return(
                        waiter,
                        cellgov_ps3_abi::cell_errors::CELL_EFAULT.into(),
                    );
                }
            } else {
                self.registry.set_syscall_return(waiter, exit_value);
            }
            self.registry
                .set_status_override(waiter, UnitStatus::Runnable);
        }
        // Inheritors route through the sync-wake path so their
        // `LwMutexWake` response repairs the user-space struct
        // (decrement waiter, set owner = inheritor, recursive_count = 1).
        if !lwmutex_inheritors.is_empty() {
            self.resolve_sync_wakes(&lwmutex_inheritors);
        }
    }

    /// Overrides replace the existing entry;
    /// [`Self::assert_response_updates_valid`] enforces the
    /// [`Lv2Dispatch::WakeAndReturn`] `response_updates` invariants.
    ///
    /// Cross-module contract: when the caller is a callback worker,
    /// `Lv2Host::dispatch_callback_return` has already transitioned
    /// its `PpuThread` to `Finished`. `is_ppu_thread_finished_for_unit`
    /// mirrors that into `UnitStatus::Finished` so the PPU loop does
    /// not fetch past the trampoline (which lands on OPD bytes and
    /// decode-faults).
    fn handle_wake_and_return(
        &mut self,
        source: UnitId,
        code: u64,
        woken_unit_ids: Vec<UnitId>,
        response_updates: Vec<(UnitId, PendingResponse)>,
        effects: Vec<Effect>,
    ) {
        let caller_space = self.spaces.space_of(source);
        self.apply_lv2_effects(&effects, caller_space);
        self.registry.set_syscall_return(source, code);
        self.assert_response_updates_valid(
            "handle_wake_and_return",
            &woken_unit_ids,
            &response_updates,
        );
        for (waiter, response) in response_updates {
            // Partial-fill refinement (e.g. EventQueueReceive
            // None -> Some): drain before re-insert so the insert
            // contract holds. Variant-tag check above guards shape.
            // A restage supersedes any timer deadline on the original
            // wait -- the cond two-hop converts a timed cond wait into
            // an untimed mutex wait.
            self.timer_wakes.cancel(waiter);
            let _ = self.syscall_responses.try_take(waiter);
            let _ = self.syscall_responses.insert(waiter, response);
        }
        self.resolve_sync_wakes(&woken_unit_ids);
        if self.lv2_host.is_ppu_thread_finished_for_unit(source) {
            self.registry
                .set_status_override(source, UnitStatus::Finished);
        }
    }

    /// Park-and-release (e.g. cond_wait): r3 is set by the eventual
    /// wake that resolves `pending`, not by this site.
    fn handle_block_and_wake(
        &mut self,
        source: UnitId,
        pending: PendingResponse,
        woken_unit_ids: Vec<UnitId>,
        response_updates: Vec<(UnitId, PendingResponse)>,
        effects: Vec<Effect>,
    ) {
        let caller_space = self.spaces.space_of(source);
        self.apply_lv2_effects(&effects, caller_space);
        self.assert_response_updates_valid(
            "handle_block_and_wake",
            &woken_unit_ids,
            &response_updates,
        );
        for (waiter, response) in response_updates {
            self.timer_wakes.cancel(waiter);
            let _ = self.syscall_responses.try_take(waiter);
            let _ = self.syscall_responses.insert(waiter, response);
        }
        self.resolve_sync_wakes(&woken_unit_ids);
        let displaced = self.syscall_responses.insert(source, pending);
        debug_assert!(
            displaced.is_none(),
            "handle_block_and_wake: source {source:?} already had a pending response: \
             {displaced:?}"
        );
        self.registry
            .set_status_override(source, UnitStatus::Blocked);
    }

    fn assert_response_updates_valid(
        &self,
        site: &'static str,
        woken_unit_ids: &[UnitId],
        response_updates: &[(UnitId, PendingResponse)],
    ) {
        if !cfg!(debug_assertions) {
            return;
        }
        check_response_updates(
            site,
            &self.syscall_responses,
            woken_unit_ids,
            response_updates,
        );
    }
}

/// `Hypercall` and `TimerFastPath` dispositions are set by the caller
/// (it knows LEV; the timer bypass skips classify entirely); every
/// other variant maps here.
fn disposition_from_request(request: &cellgov_lv2::Lv2Request) -> TracedSyscallDisposition {
    match request {
        cellgov_lv2::Lv2Request::Unsupported { .. } => TracedSyscallDisposition::Unsupported,
        cellgov_lv2::Lv2Request::UnresolvedImport { .. } => {
            TracedSyscallDisposition::UnresolvedImport
        }
        cellgov_lv2::Lv2Request::Malformed { .. } => TracedSyscallDisposition::Malformed,
        cellgov_lv2::Lv2Request::Hypercall { .. } => TracedSyscallDisposition::Hypercall,
        _ => TracedSyscallDisposition::Implemented,
    }
}

pub(crate) fn check_response_updates(
    site: &'static str,
    table: &crate::syscall_table::SyscallResponseTable,
    woken_unit_ids: &[UnitId],
    response_updates: &[(UnitId, PendingResponse)],
) {
    for (waiter, update) in response_updates {
        // A legitimate update targets a unit being woken now, or a
        // still-parked unit whose staged response is being replaced --
        // the cond two-hop restages a contended waiter's cond-wait
        // response as the mutex-grant response without waking it.
        // Neither-woken-nor-parked means the update can never deliver.
        assert!(
            woken_unit_ids.contains(waiter) || table.peek(*waiter).is_some(),
            "{site}: response_updates entry for {waiter:?} is neither in woken_unit_ids \
             nor parked with a pending response",
        );
        // ReturnCode is the universal cancel/timeout override and
        // may replace any prior variant; the tag invariant only
        // constrains payload-carrying refinements.
        if matches!(update, PendingResponse::ReturnCode { .. }) {
            continue;
        }
        if let Some(existing) = table.peek(*waiter) {
            // sys_event_flag_cancel refines a parked eflag-wait
            // response into its ECANCELED counterpart; every other
            // payload-carrying update must match the parked variant.
            if matches!(
                (existing, update),
                (
                    PendingResponse::EventFlagWake { .. },
                    PendingResponse::EventFlagCancelWake { .. }
                )
            ) {
                continue;
            }
            assert_eq!(
                existing.variant_tag(),
                update.variant_tag(),
                "{site}: response_updates variant mismatch for {waiter:?} \
                 (existing tag {}, update tag {})",
                existing.variant_tag(),
                update.variant_tag(),
            );
        }
    }
}

#[cfg(test)]
#[path = "tests/lv2_dispatch_tests.rs"]
mod tests;
