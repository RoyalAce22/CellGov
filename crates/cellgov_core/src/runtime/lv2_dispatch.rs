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

use super::types::RuntimeMode;
use super::{
    trace_bridge::{traced_invariant_break_reason, MemoryView},
    Runtime,
};

impl Runtime {
    /// Apply an `Lv2Dispatch` effects batch by **direct commit**,
    /// bypassing the commit pipeline's [`StagingMemory`].
    ///
    /// - `SharedWriteIntent` -> [`GuestMemory::apply_commit`]; the
    ///   memory subset validates and commits all-or-none, and a
    ///   failure logs a `dispatch.lv2_effect_apply_failed` invariant
    ///   break instead of landing any write.
    /// - `MailboxSend` -> `force_send` plus a Blocked -> Runnable
    ///   override on any unit blocked on the matching `UnitId`.
    /// - `RsxFlipRequest` -> [`crate::rsx::flip::RsxFlipState::request_flip`];
    ///   the flip transitions WAITING -> DONE on the next
    ///   `commit_step` boundary, not during the dispatching batch.
    /// - Every other variant is silently dropped: LV2 handlers must
    ///   not emit effects the LV2 surface cannot apply (e.g.
    ///   `RsxLabelWrite`, `DmaEnqueue`); those belong to the
    ///   unit-effect path through `commit_pipeline.process`.
    ///
    /// [`StagingMemory`]: cellgov_mem::StagingMemory
    /// [`GuestMemory::apply_commit`]: cellgov_mem::GuestMemory::apply_commit
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
    /// `Runtime::commit_step` drains staged unit effects FIRST and
    /// calls this function SECOND, so an LV2 write deterministically
    /// lands AFTER any same-batch unit store to the same range. The
    /// ordering is structural (call order), not a staging-buffer
    /// tiebreak; the `(PriorityClass, source_time, source)` triple
    /// is inert on this path and carried as a well-formedness
    /// invariant only.
    ///
    /// # Cross-module contract
    ///
    /// LV2 handlers must not emit a non-memory effect whose
    /// semantics depend on a co-batched `SharedWriteIntent` having
    /// committed; a `debug_assert!` below traps that condition.
    pub(super) fn apply_lv2_effects(&mut self, effects: &[Effect]) {
        let memory_failure = self.validate_lv2_memory_subset(effects);
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
                     length={length}: {err}; memory subset rolled back, non-memory \
                     effects still applied",
                ),
            );
        }
        for effect in effects {
            // Exhaustive match with no wildcard: a new variant on
            // cellgov_effects::Effect must add an arm here (compile
            // error otherwise). This is the "classify, never silently
            // drop" discipline applied to effect application -- a
            // future LV2 handler that emits an unmodeled variant
            // surfaces loudly via the unsupported-arm log_invariant_break
            // rather than vanishing through a `_ => {}` catch-all.
            match effect {
                Effect::SharedWriteIntent { range, bytes, .. } => {
                    if memory_failure.is_some() {
                        continue;
                    }
                    self.memory.apply_commit(*range, bytes.bytes()).expect(
                        "validate_lv2_memory_subset called GuestMemory::validate_write -- \
                         the same predicate apply_commit uses internally -- so this Err \
                         path is structurally unreachable",
                    );
                    // Tripwire witness: a future refactor that routes
                    // this through StagingMemory drops the increment;
                    // the integration test below asserts this stays
                    // nonzero on a real FIFO_SETUP commit_step.
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
                // Unsupported-from-LV2 variants. Each names the
                // expected origin so a reader can disambiguate "LV2
                // handler emitted by mistake" from "execution-unit
                // path leaked through". All route through
                // log_invariant_break under the
                // `runtime.apply_lv2_effects_unsupported_*` tag prefix
                // for grep-separability from genuine internal-invariant
                // failures (matches the cellgov_lv2 `_unsupported_`
                // convention for honest not-implemented arms).
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

    /// Pre-validate the `SharedWriteIntent` subset of `effects` via
    /// `GuestMemory::validate_write` (the predicate `apply_commit`
    /// itself calls). Returns the first failing intent or `None`.
    fn validate_lv2_memory_subset(
        &self,
        effects: &[Effect],
    ) -> Option<(cellgov_mem::ByteRange, MemError)> {
        for effect in effects {
            if let Effect::SharedWriteIntent { range, bytes, .. } = effect {
                if let Err(err) = self.memory.validate_write(*range, bytes.len()) {
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

        // Emit the entry record before any state mutation; FaultDriven
        // suppresses trace writes per the existing convention.
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

        // Timer fast-path: advance simulated clock without yielding
        // through `Lv2Host::dispatch`. Other PPU threads observe the
        // new time on their next read.
        match num {
            TIMER_USLEEP => {
                let usec = args8[0];
                self.advance_guest_time_by_us(usec);
            }
            TIMER_SLEEP => {
                let seconds = args8[0];
                let usec = seconds.saturating_mul(1_000_000);
                self.advance_guest_time_by_us(usec);
            }
            _ => unreachable!("is_timer_fast_path implies num is TIMER_USLEEP or TIMER_SLEEP"),
        }
        self.registry.set_syscall_return(source, 0);
    }

    /// Saturates at `u64::MAX` ticks.
    fn advance_guest_time_by_us(&mut self, usec: u64) {
        // 1 tick == 1 ns per cellgov_time::SIMULATED_INSTRUCTIONS_PER_SECOND.
        let delta_ticks = usec.saturating_mul(1_000);
        let new_raw = self.time.raw().saturating_add(delta_ticks);
        self.time = cellgov_time::GuestTicks::new(new_raw);
    }

    pub(crate) fn dispatch_lv2_request(
        &mut self,
        request: cellgov_lv2::Lv2Request,
        source: UnitId,
    ) {
        let is_process_exit = matches!(request, cellgov_lv2::Lv2Request::ProcessExit { .. });
        let dispatch = self.lv2_host.dispatch(
            request,
            source,
            &MemoryView {
                memory: &self.memory,
                current_tick: self.time,
            },
        );
        // Always drain so the buffer stays bounded; only emit trace
        // records under modes that write a trace stream. FaultDriven
        // consults `invariant_break_count` via the boot summary.
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
        // Apply shm region-install requests before the dispatch's effects
        // commit: a 334 that mints a fresh region and an effect targeting
        // that region in the same dispatch would otherwise hit
        // `CommitError::OutOfRange` at pre-validation. Collect into a
        // local so the borrow on `self.lv2_host` releases before
        // touching `self.memory`.
        let region_installs: Vec<(u64, usize)> =
            self.lv2_host.drain_pending_region_installs().collect();
        for (addr, size) in region_installs {
            self.memory
                .install_region(addr, size, "shm", cellgov_mem::PageSize::Page64K)
                .expect(
                    "mmapper handle table guarantees disjointness against the existing layout; \
                     overlap here means 334 dispatch let a contradiction through",
                );
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
                pending, effects, ..
            } => {
                self.handle_block(source, pending, effects);
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
                pending,
                woken_unit_ids,
                response_updates,
                effects,
                ..
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
        self.apply_lv2_effects(&effects);
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
        self.apply_lv2_effects(&effects);
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
        self.apply_lv2_effects(&effects);
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
        self.apply_lv2_effects(&effects);
        self.registry
            .set_status_override(source, UnitStatus::Finished);
        for waiter in woken_unit_ids {
            let pending = self.syscall_responses.try_take(waiter);
            if let Some(PendingResponse::PpuThreadJoin { status_out_ptr, .. }) = pending {
                self.commit_bytes_at(status_out_ptr as u64, &exit_value.to_be_bytes());
                self.registry.set_syscall_return(waiter, 0);
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

    /// Overrides replace (not merge) the existing entry;
    /// [`Self::assert_response_updates_valid`] enforces that every
    /// updated unit is in `woken_unit_ids` and each update's variant
    /// matches the existing entry.
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
        self.apply_lv2_effects(&effects);
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
        self.apply_lv2_effects(&effects);
        self.assert_response_updates_valid(
            "handle_block_and_wake",
            &woken_unit_ids,
            &response_updates,
        );
        for (waiter, response) in response_updates {
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

    /// Debug-only: every updated unit is in the wake set, and each
    /// payload-carrying update's variant matches the existing entry.
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

/// Map a classified `Lv2Request` to its [`TracedSyscallDisposition`].
///
/// `Hypercall` is set by the caller (it knows LEV); `TimerFastPath` is
/// set by the caller (the timer bypass skips classify entirely). Every
/// other variant routes through this helper.
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
        assert!(
            woken_unit_ids.contains(waiter),
            "{site}: response_updates entry for {waiter:?} is not in woken_unit_ids",
        );
        // ReturnCode is the universal cancel/timeout override and
        // may replace any prior variant; the tag invariant only
        // constrains payload-carrying refinements.
        if matches!(update, PendingResponse::ReturnCode { .. }) {
            continue;
        }
        if let Some(existing) = table.peek(*waiter) {
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
