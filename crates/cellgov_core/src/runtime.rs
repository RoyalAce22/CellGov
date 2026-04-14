//! The `Runtime` struct -- top-level orchestration.
//!
//! The runtime pipeline has ten steps. [`Runtime::step`] drives
//! steps 1-3 and 9; [`Runtime::commit_step`] drives steps 4-8 and
//! 10:
//!
//! 1. Select a runnable unit deterministically (via [`crate::Scheduler`])
//! 2. Grant the configured per-step budget
//! 3. Run the unit until it yields
//! 4. Collect emitted effects
//! 5. Validate effects
//! 6. Stage commit batch
//! 7. Apply commit batch to shared visible state
//! 8. Inject resulting events/wakeups (DMA completions, LV2 syscall
//!    dispatch, block/wake transitions)
//! 9. Advance guest time by the unit's consumed budget
//! 10. Trace every decision
//!
//! When a unit yields with `YieldReason::Syscall`, `commit_step`
//! classifies the args, dispatches through `Lv2Host`, and handles the
//! response: `Immediate` stores the return code for the next step,
//! `RegisterSpu` constructs new SPU units via the pluggable
//! `SpuFactory`, and `Block` stores a `PendingResponse` in the
//! `SyscallResponseTable` and transitions the caller to `Blocked`.
//!
//! ## Determinism contract
//!
//! Everything `Runtime` does is a pure function of `(its own state,
//! the registry contents, the scheduler decisions)`. No host time, no
//! thread scheduling, no `HashMap` iteration. The scheduler is
//! deterministic by trait contract; the registry uses `BTreeMap`; the
//! time advancement policy is identity.
//!
//! ## Deadlock detector
//!
//! The runner always carries a max-steps / deadlock detector. Stalls
//! fail cleanly; they never hang the suite. `Runtime` enforces this
//! at the runtime layer rather than the runner
//! layer so every consumer benefits, including unit tests. A
//! configurable `max_steps` cap is checked at the start of each
//! `step()` call; exceeding it returns [`StepError::MaxStepsExceeded`]
//! instead of looping forever.
//!
//! ## Tracing
//!
//! Pipeline step 10 ("trace every decision") is implemented here. The
//! runtime owns a [`TraceWriter`] and emits structured binary records
//! for every scheduling and commit decision it makes:
//!
//! - [`TraceRecord::UnitScheduled`] before the unit runs, carrying the
//!   selected unit, the granted budget, and the pre-step time/epoch.
//! - [`TraceRecord::StepCompleted`] after `run_until_yield` returns,
//!   carrying the yield reason, consumed budget, and post-step time.
//! - [`TraceRecord::EffectEmitted`] for every effect the unit produced,
//!   in stable emission order, with `sequence` running 0..N inside the
//!   step. Per-effect payloads are not yet recorded; the kind alone is
//!   enough for replay tooling to verify the sequence.
//! - [`TraceRecord::CommitApplied`] after [`Runtime::commit_step`]
//!   processes a step's effects, carrying the per-batch counts and the
//!   post-commit epoch.
//! - [`TraceRecord::UnitBlocked`] for each unit whose status was
//!   overridden to `Blocked` during the commit (empty mailbox receive,
//!   wait-on-event).
//! - [`TraceRecord::UnitWoken`] for each unit whose status was
//!   overridden to `Runnable` during the commit (wake effect, DMA
//!   completion).
//! - [`TraceRecord::StateHashCheckpoint`] for all four checkpoint
//!   kinds (committed memory, runnable queue, unit status, sync state)
//!   after every commit, so replay tooling can compare post-commit
//!   states across runs.
//!
//! The trace format is binary, not text. Tests pull
//! the bytes out via [`Runtime::trace`] and feed them to the
//! [`cellgov_trace::TraceReader`] for assertions. Text rendering is a
//! downstream tool over the same binary buffer.

use crate::commit::{CommitContext, CommitError, CommitOutcome, CommitPipeline};
use crate::registry::{RegisteredUnit, UnitRegistry};
use crate::scheduler::{RoundRobinScheduler, Scheduler};
use crate::syscall_table::SyscallResponseTable;
use cellgov_dma::{DmaLatencyModel, DmaQueue, FixedLatency};
use cellgov_event::UnitId;
use cellgov_exec::{ExecutionContext, ExecutionStepResult, UnitStatus, YieldReason};
use cellgov_lv2::{Lv2Dispatch, Lv2Host, Lv2Runtime, PendingResponse, SpuInitState};
use cellgov_mem::GuestMemory;
use cellgov_sync::{MailboxRegistry, SignalRegistry};
use cellgov_time::{Budget, Epoch, GuestTicks};
use cellgov_trace::{
    HashCheckpointKind, StateHash, TraceRecord, TraceWriter, TracedBlockReason, TracedEffectKind,
    TracedWakeReason, TracedYieldReason,
};

/// One pass of the runtime pipeline as observed from outside.
///
/// Returned by [`Runtime::step`] on success. Carries the selected
/// unit, the unit's step result (with emitted effects in stable
/// order), and the runtime's time/epoch values *after* the step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeStep {
    /// Which unit was selected and run.
    pub unit: UnitId,
    /// What the unit returned from `run_until_yield`. Effects are in
    /// the order the unit emitted them; the runtime never reorders.
    pub result: ExecutionStepResult,
    /// Guest time after this step's consumed budget was applied.
    pub time_after: GuestTicks,
    /// Epoch after this step. Currently only advances at commit
    /// boundaries, which the commit pipeline owns; `step()` does not
    /// advance the epoch and the value is unchanged from before the
    /// step.
    pub epoch_after: Epoch,
}

/// Why a [`Runtime::step`] call could not produce a step.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepError {
    /// No unit in the registry is currently runnable. The scheduler
    /// has nothing to schedule. The runtime is not necessarily done
    /// (a future event may wake a blocked unit) -- the caller decides
    /// whether to retry, advance time, or treat this as a stall.
    NoRunnableUnit,
    /// The runtime has already executed `max_steps` steps. Further
    /// stepping is the deadlock detector firing; the caller must
    /// abort the run rather than retry.
    MaxStepsExceeded,
    /// The runtime tried to advance guest time past `u64::MAX`. This
    /// is a runtime invariant violation in any realistic scenario;
    /// surfaced as an error rather than a panic so tests can assert
    /// on it.
    TimeOverflow,
}

/// The top-level runtime.
///
/// Composes the registry, scheduler, committed memory, guest time,
/// and epoch into a single object that drives one step at a time via
/// [`Runtime::step`]. The scheduler is pluggable via
/// [`Runtime::set_scheduler`]; defaults to [`RoundRobinScheduler`].
/// Factory that constructs an SPU execution unit from an init state.
/// The runtime calls this when `Lv2Dispatch::RegisterSpu` fires.
/// The factory receives the `UnitId` the registry allocated and the
/// `SpuInitState` the LV2 host produced; it returns a boxed unit
/// ready to run.
pub type SpuFactory = Box<dyn Fn(UnitId, SpuInitState) -> Box<dyn RegisteredUnit>>;

/// Controls the runtime's overhead profile: which trace records are
/// emitted and whether state-hash checkpoints are computed at commit
/// boundaries.
///
/// Replaces the loose `skip_hash_checkpoints` flag with a single
/// configuration point that is harder to misconfigure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeMode {
    /// Trace off, hash checkpoints off. Minimal per-step bookkeeping.
    /// Default for `run-game`.
    FaultDriven,
    /// Trace on (commits + block/wake only), hash checkpoints on.
    /// For microtest replay and oracle comparison.
    DeterminismCheck,
    /// All trace records, all hash checkpoints.
    /// For exploration and debugging.
    FullTrace,
}

/// Deterministic step-loop runtime over guest memory and registered units.
pub struct Runtime {
    pub(crate) registry: UnitRegistry,
    mailbox_registry: MailboxRegistry,
    signal_registry: SignalRegistry,
    dma_queue: DmaQueue,
    dma_latency: Box<dyn DmaLatencyModel>,
    lv2_host: Lv2Host,
    syscall_responses: SyscallResponseTable,
    spu_factory: Option<SpuFactory>,
    scheduler: Box<dyn Scheduler>,
    commit_pipeline: CommitPipeline,
    pub(crate) memory: GuestMemory,
    time: GuestTicks,
    epoch: Epoch,
    budget_per_step: Budget,
    steps_taken: usize,
    max_steps: usize,
    trace: TraceWriter,
    /// The unit selected by the most recent `step()` call. Used by
    /// `commit_step()` to attribute the commit record to the unit that
    /// produced the batch -- there is one commit batch per unit yield,
    /// so this is always the right unit.
    last_scheduled_unit: Option<UnitId>,
    /// HLE NID table: maps HLE index -> NID for dispatch of HLE calls
    /// that need non-trivial behavior (e.g., TLS init, mutex create).
    hle_nids: std::collections::BTreeMap<u32, u32>,
    /// Bump allocator pointer for _sys_malloc HLE. Points to the next
    /// free address in guest memory. Allocations are never freed.
    pub(crate) hle_heap_ptr: u32,
    /// Monotonic kernel-object ID counter for HLE-created primitives
    /// (lwmutex sleep_queue, etc.). Starts above zero so a zero-initialized
    /// guest field is distinguishable from a legitimate allocated ID.
    pub(crate) hle_next_id: u32,
    /// Controls trace and hash checkpoint overhead. Defaults to
    /// `FullTrace` (all records, all hashes). `FaultDriven` disables
    /// both; `DeterminismCheck` enables hashes and commit-level trace.
    mode: RuntimeMode,
    /// Monotonic counter over per-instruction state hashes. Each
    /// entry drained from a unit's `drain_retired_state_hashes` is
    /// stamped with this counter, then the counter is incremented.
    /// The counter is orthogonal to `steps_taken` (which counts
    /// `run_until_yield` invocations, not individual retired
    /// instructions).
    per_step_index: u64,
    /// Separate trace sink for zoom-in `PpuStateFull` records. Kept
    /// distinct from the main `trace` writer so the per-step stream
    /// stays homogeneous: every record in `trace` is fixed-size
    /// `PpuStateHash`, every record in `zoom_trace` is `PpuStateFull`.
    /// The CLI extracts this via `Runtime::zoom_trace`.
    zoom_trace: TraceWriter,
}

impl Runtime {
    /// Construct a runtime over the given memory, with the given
    /// per-step budget grant and the given max-steps cap. The
    /// scheduler starts at the beginning of the registry; time and
    /// epoch start at zero; no units are registered.
    ///
    /// Use [`Runtime::registry_mut`] to register units before stepping.
    pub fn new(memory: GuestMemory, budget_per_step: Budget, max_steps: usize) -> Self {
        Self::with_trace_writer(memory, budget_per_step, max_steps, TraceWriter::new())
    }

    /// Construct a runtime with a caller-supplied [`TraceWriter`].
    ///
    /// Used by tests and the testkit runner to install a writer with a
    /// specific level filter (for example, commits + hashes only) so the
    /// high-volume categories can be filtered, exercising that contract
    /// end-to-end. Behaviorally identical to [`Runtime::new`] otherwise.
    pub fn with_trace_writer(
        memory: GuestMemory,
        budget_per_step: Budget,
        max_steps: usize,
        trace: TraceWriter,
    ) -> Self {
        Self {
            registry: UnitRegistry::new(),
            mailbox_registry: MailboxRegistry::new(),
            signal_registry: SignalRegistry::new(),
            dma_queue: DmaQueue::new(),
            dma_latency: Box::new(FixedLatency::new(10)),
            lv2_host: Lv2Host::new(),
            syscall_responses: SyscallResponseTable::new(),
            spu_factory: None,
            scheduler: Box::new(RoundRobinScheduler::new()),
            commit_pipeline: CommitPipeline::new(),
            memory,
            time: GuestTicks::ZERO,
            epoch: Epoch::ZERO,
            budget_per_step,
            steps_taken: 0,
            max_steps,
            trace,
            last_scheduled_unit: None,
            hle_nids: std::collections::BTreeMap::new(),
            hle_heap_ptr: 0,
            hle_next_id: 0x8000_0001,
            mode: RuntimeMode::FullTrace,
            per_step_index: 0,
            zoom_trace: TraceWriter::new(),
        }
    }

    /// Drive the commit pipeline for a previously-returned step result.
    ///
    /// Validates, stages, and applies the effects in `result` against
    /// committed memory, then advances the epoch. The epoch advances
    /// on every commit boundary -- including validation
    /// failures, since the step's set of effects is "closed" either
    /// way -- so a `Err` return still mutates `self.epoch`.
    ///
    /// Fault rule and atomic-batch semantics are inherited
    /// from [`CommitPipeline::process`]; this method is the
    /// runtime-level seam that also owns epoch advancement.
    pub fn commit_step(
        &mut self,
        result: &ExecutionStepResult,
    ) -> Result<CommitOutcome, CommitError> {
        let mut ctx = CommitContext {
            memory: &mut self.memory,
            units: &mut self.registry,
            mailboxes: &mut self.mailbox_registry,
            signals: &mut self.signal_registry,
            dma_queue: &mut self.dma_queue,
            dma_latency: self.dma_latency.as_ref(),
            now: self.time,
        };
        let mut outcome = self.commit_pipeline.process(result, &mut ctx);

        let source = self.last_scheduled_unit.unwrap_or_else(|| UnitId::new(0));
        if result.yield_reason == YieldReason::Syscall {
            self.dispatch_syscall(result, source);
        }

        self.epoch = self.epoch.next().expect("epoch overflow");
        let due = self.fire_dma_completions();
        if let Ok(ref mut o) = outcome {
            o.dma_completions_fired = due.len();
        }

        if result.yield_reason == YieldReason::Finished {
            self.resolve_join_wakes(source);
        }

        // Trace pipeline step 7 (and the validation rejection edge): one
        // CommitApplied record per commit boundary, carrying the
        // post-commit epoch. On validation failure, rejection surfaces
        // as a fault on the originating unit, so
        // we record fault_discarded = true with zero counts -- the
        // batch is closed, just empty.
        //
        // Attribution: there is one commit batch per unit yield, so
        // `source` (defined above) is the unit that produced the batch.
        // Commit-level trace records fire in DeterminismCheck and
        // FullTrace modes but not FaultDriven.
        if self.mode != RuntimeMode::FaultDriven {
            let record = match &outcome {
                Ok(o) => TraceRecord::CommitApplied {
                    unit: source,
                    writes_committed: o.writes_committed as u32,
                    effects_deferred: o.effects_deferred as u32,
                    fault_discarded: o.fault_discarded,
                    epoch_after: self.epoch,
                },
                Err(_) => TraceRecord::CommitApplied {
                    unit: source,
                    writes_committed: 0,
                    effects_deferred: 0,
                    fault_discarded: true,
                    epoch_after: self.epoch,
                },
            };
            self.trace.record(&record);

            if let Ok(ref o) = outcome {
                for &(unit, ref reason) in &o.blocked_units {
                    let traced_reason = match reason {
                        crate::commit::BlockReason::MailboxEmpty => TracedBlockReason::MailboxEmpty,
                        crate::commit::BlockReason::WaitOnEvent => TracedBlockReason::WaitOnEvent,
                    };
                    self.trace.record(&TraceRecord::UnitBlocked {
                        unit,
                        reason: traced_reason,
                    });
                }
                for &unit in &o.woken_units {
                    self.trace.record(&TraceRecord::UnitWoken {
                        unit,
                        reason: TracedWakeReason::WakeEffect,
                    });
                }
            }
            for (c, _) in &due {
                self.trace.record(&TraceRecord::UnitWoken {
                    unit: c.issuer(),
                    reason: TracedWakeReason::DmaCompletion,
                });
            }
        }

        // State hash checkpoints. Four kinds:
        // committed memory, runnable queue, sync state, and unit
        // status. All four are emitted here, taken AFTER the commit
        // (including DMA completion firing) so replay tooling sees
        // post-commit state. Skipped when hash checkpoints are
        // disabled (large guest memories where O(N) hashing per
        // step is prohibitive).
        if self.mode != RuntimeMode::FaultDriven {
            let mem_hash = StateHash::new(self.memory.content_hash());
            self.trace.record(&TraceRecord::StateHashCheckpoint {
                kind: HashCheckpointKind::CommittedMemory,
                hash: mem_hash,
            });
            let rq_hash = StateHash::new(self.registry.runnable_queue_hash());
            self.trace.record(&TraceRecord::StateHashCheckpoint {
                kind: HashCheckpointKind::RunnableQueue,
                hash: rq_hash,
            });
            let status_hash = StateHash::new(self.registry.status_hash());
            self.trace.record(&TraceRecord::StateHashCheckpoint {
                kind: HashCheckpointKind::UnitStatus,
                hash: status_hash,
            });
            let sync_hash = StateHash::new(self.sync_state_hash());
            self.trace.record(&TraceRecord::StateHashCheckpoint {
                kind: HashCheckpointKind::SyncState,
                hash: sync_hash,
            });
        }

        outcome
    }

    // ------------------------------------------------------------------
    // Private helpers extracted from commit_step for SRP.
    // ------------------------------------------------------------------

    /// Apply effects produced by an LV2 dispatch. Handles
    /// SharedWriteIntent (memory commit) and MailboxSend (FIFO push
    /// + blocked-SPU wake) uniformly across all dispatch variants.
    fn apply_lv2_effects(&mut self, effects: &[cellgov_effects::Effect]) {
        for effect in effects {
            match effect {
                cellgov_effects::Effect::SharedWriteIntent { range, bytes, .. } => {
                    let _ = self.memory.apply_commit(*range, bytes.bytes());
                }
                cellgov_effects::Effect::MailboxSend {
                    mailbox, message, ..
                } => {
                    if let Some(mbox) = self.mailbox_registry.get_mut(*mailbox) {
                        mbox.send(message.raw());
                    }
                    let target = UnitId::new(mailbox.raw());
                    if self.registry.effective_status(target) == Some(UnitStatus::Blocked) {
                        self.registry
                            .set_status_override(target, UnitStatus::Runnable);
                    }
                }
                _ => {}
            }
        }
    }

    /// Classify a syscall yield, dispatch through the LV2 host, and
    /// handle the result (immediate return, SPU registration, or block).
    fn dispatch_syscall(&mut self, result: &ExecutionStepResult, source: UnitId) {
        let Some(args) = &result.syscall_args else {
            return;
        };

        // HLE import stubs use syscall numbers >= 0x10000.
        if args[0] >= 0x10000 {
            let hle_index = (args[0] - 0x10000) as u32;
            let nid = self.hle_nids.get(&hle_index).copied().unwrap_or(0);
            self.dispatch_hle(source, nid, args);
            return;
        }

        let request = cellgov_lv2::request::classify(
            args[0],
            &[
                args[1], args[2], args[3], args[4], args[5], args[6], args[7], args[8],
            ],
        );
        let is_process_exit = matches!(request, cellgov_lv2::Lv2Request::ProcessExit { .. });
        let dispatch = self
            .lv2_host
            .dispatch(request, source, &MemoryView(&self.memory));
        match dispatch {
            Lv2Dispatch::Immediate { code, effects } => {
                self.apply_lv2_effects(&effects);
                if is_process_exit {
                    let all_ids: Vec<UnitId> = self.registry.ids().collect();
                    for uid in &all_ids {
                        self.registry
                            .set_status_override(*uid, UnitStatus::Finished);
                        self.lv2_host.notify_spu_finished(*uid);
                        self.syscall_responses.take(*uid);
                    }
                } else {
                    self.registry.set_syscall_return(source, code);
                }
            }
            Lv2Dispatch::RegisterSpu {
                inits,
                effects,
                code,
            } => {
                self.apply_lv2_effects(&effects);
                if let Some(factory) = &self.spu_factory {
                    for init in inits {
                        let gid = init.group_id;
                        let slot = init.slot;
                        let uid = self
                            .registry
                            .register_dynamic(&|id| factory(id, init.clone()));
                        self.lv2_host.record_spu(uid, gid, slot);
                        self.mailbox_registry
                            .register_at(cellgov_sync::MailboxId::new(uid.raw()));
                    }
                }
                self.registry.set_syscall_return(source, code);
            }
            Lv2Dispatch::Block {
                pending, effects, ..
            } => {
                self.apply_lv2_effects(&effects);
                self.syscall_responses.insert(source, pending);
                self.registry
                    .set_status_override(source, UnitStatus::Blocked);
            }
        }
    }

    /// Pop and apply DMA completions whose modeled time has arrived.
    /// Returns the list of fired completions for trace recording.
    fn fire_dma_completions(&mut self) -> Vec<(cellgov_dma::DmaCompletion, Option<Vec<u8>>)> {
        let due = self.dma_queue.pop_due(self.time);
        for (c, payload) in &due {
            let bytes = if let Some(data) = payload {
                data.clone()
            } else if let Some(src) = self.memory.read(c.source()) {
                src.to_vec()
            } else {
                continue;
            };
            let _ = self.memory.apply_commit(c.destination(), &bytes);
            self.registry
                .set_status_override(c.issuer(), UnitStatus::Runnable);
        }
        due
    }

    /// When an SPU finishes, notify the LV2 host. If the group is
    /// fully finished, find and wake the PPU blocked on that group's
    /// join with its pending response.
    fn resolve_join_wakes(&mut self, source: UnitId) {
        let Some(finished_group) = self.lv2_host.notify_spu_finished(source) else {
            return;
        };
        let waiters: Vec<UnitId> = self.syscall_responses.pending_ids().collect();
        for waiter_id in waiters {
            let is_match = self
                .syscall_responses
                .peek(waiter_id)
                .map(|p| {
                    matches!(p, PendingResponse::ThreadGroupJoin { group_id, .. } if *group_id == finished_group)
                })
                .unwrap_or(false);
            if !is_match {
                continue;
            }
            if let Some(pending) = self.syscall_responses.take(waiter_id) {
                match &pending {
                    PendingResponse::ThreadGroupJoin {
                        code,
                        cause_ptr,
                        status_ptr,
                        cause,
                        status,
                        ..
                    } => {
                        self.registry.set_syscall_return(waiter_id, *code);
                        self.registry
                            .set_status_override(waiter_id, UnitStatus::Runnable);
                        if let Some(range) = cellgov_mem::ByteRange::new(
                            cellgov_mem::GuestAddr::new(*cause_ptr as u64),
                            4,
                        ) {
                            let _ = self.memory.apply_commit(range, &cause.to_be_bytes());
                        }
                        if let Some(range) = cellgov_mem::ByteRange::new(
                            cellgov_mem::GuestAddr::new(*status_ptr as u64),
                            4,
                        ) {
                            let _ = self.memory.apply_commit(range, &status.to_be_bytes());
                        }
                    }
                    PendingResponse::ReturnCode { code } => {
                        self.registry.set_syscall_return(waiter_id, *code);
                        self.registry
                            .set_status_override(waiter_id, UnitStatus::Runnable);
                    }
                }
            }
        }
    }

    /// Borrow the binary trace buffer accumulated so far.
    ///
    /// Tests and the testkit runner pull bytes via
    /// [`TraceWriter::bytes`] and feed them to
    /// [`cellgov_trace::TraceReader`] for assertions. The runtime never
    /// reorders records: the buffer is exactly the sequence of decisions
    /// it made, in the order it made them.
    #[inline]
    pub fn trace(&self) -> &TraceWriter {
        &self.trace
    }

    /// Borrow the 9G zoom-in trace writer. Empty unless a unit had a
    /// zoom-in window configured during one of the runs that fed
    /// `step()`. Kept distinct from `trace()` so the per-step stream
    /// stays homogeneous.
    #[inline]
    pub fn zoom_trace(&self) -> &TraceWriter {
        &self.zoom_trace
    }

    /// Borrow the unit registry.
    #[inline]
    pub fn registry(&self) -> &UnitRegistry {
        &self.registry
    }

    /// Mutably borrow the unit registry. The construction-time entry
    /// point for registering units; not intended to be called from
    /// inside a `step()`.
    #[inline]
    pub fn registry_mut(&mut self) -> &mut UnitRegistry {
        &mut self.registry
    }

    /// Borrow the mailbox registry. Folded into the per-commit
    /// `SyncState` hash checkpoint; future slices wire `MailboxSend`
    /// and `MailboxReceiveAttempt` effects through this seam.
    #[inline]
    pub fn mailbox_registry(&self) -> &MailboxRegistry {
        &self.mailbox_registry
    }

    /// Mutably borrow the mailbox registry. The construction-time
    /// entry point for registering mailboxes; not intended to be
    /// called from inside a `step()`.
    #[inline]
    pub fn mailbox_registry_mut(&mut self) -> &mut MailboxRegistry {
        &mut self.mailbox_registry
    }

    /// Borrow the signal-notification register registry. Folded into
    /// the per-commit `SyncState` hash checkpoint via
    /// [`Runtime::sync_state_hash`].
    #[inline]
    pub fn signal_registry(&self) -> &SignalRegistry {
        &self.signal_registry
    }

    /// Mutably borrow the signal-notification register registry. The
    /// construction-time entry point for registering signal registers;
    /// not intended to be called from inside a `step()`.
    #[inline]
    pub fn signal_registry_mut(&mut self) -> &mut SignalRegistry {
        &mut self.signal_registry
    }

    /// Borrow the LV2 host model.
    #[inline]
    pub fn lv2_host(&self) -> &Lv2Host {
        &self.lv2_host
    }

    /// Mutably borrow the LV2 host model. The construction-time entry
    /// point for registering images and configuring the host; not
    /// intended to be called from inside a `step()`.
    #[inline]
    pub fn lv2_host_mut(&mut self) -> &mut Lv2Host {
        &mut self.lv2_host
    }

    /// Install an SPU factory. The runtime calls this factory when
    /// `Lv2Dispatch::RegisterSpu` fires during `commit_step`. The
    /// test harness or CLI sets it at setup time.
    pub fn set_spu_factory<F>(&mut self, factory: F)
    where
        F: Fn(UnitId, SpuInitState) -> Box<dyn RegisteredUnit> + 'static,
    {
        self.spu_factory = Some(Box::new(factory));
    }

    /// Borrow the syscall response table.
    #[inline]
    pub fn syscall_responses(&self) -> &SyscallResponseTable {
        &self.syscall_responses
    }

    /// Mutably borrow the syscall response table.
    #[inline]
    pub fn syscall_responses_mut(&mut self) -> &mut SyscallResponseTable {
        &mut self.syscall_responses
    }

    /// Borrow the DMA completion queue.
    #[inline]
    pub fn dma_queue(&self) -> &DmaQueue {
        &self.dma_queue
    }

    /// Replace the scheduler. The explorer uses this to inject a
    /// prescribed schedule that forces a specific unit ordering.
    pub fn set_scheduler<S: Scheduler + 'static>(&mut self, scheduler: S) {
        self.scheduler = Box::new(scheduler);
    }

    /// Register HLE NID mappings for dispatch. Maps HLE index -> NID
    /// so the runtime can dispatch specific HLE functions (TLS init, etc.).
    pub fn set_hle_nids(&mut self, nids: std::collections::BTreeMap<u32, u32>) {
        self.hle_nids = nids;
    }

    /// Set the base address for the HLE bump allocator (_sys_malloc).
    pub fn set_hle_heap_base(&mut self, base: u32) {
        self.hle_heap_ptr = base;
    }

    /// Set the runtime mode controlling trace and hash checkpoint
    /// overhead.
    pub fn set_mode(&mut self, mode: RuntimeMode) {
        self.mode = mode;
    }

    /// Current runtime mode.
    pub fn mode(&self) -> RuntimeMode {
        self.mode
    }

    /// Split-borrow both registries at once. Used by the testkit
    /// runner so a fixture's setup callback can register a mailbox
    /// and a unit that targets it in one call without fighting the
    /// borrow checker.
    #[inline]
    pub fn registries_mut(&mut self) -> (&mut UnitRegistry, &mut MailboxRegistry) {
        (&mut self.registry, &mut self.mailbox_registry)
    }

    /// Combined hash of every sync source the runtime owns: mailbox
    /// queues, signal-notification registers, and LV2 host state.
    /// Computed by FNV-1a-merging the per-source hashes in a fixed
    /// source order so the result is deterministic and stable across
    /// runs of the same scenario. Replay tooling compares pairs of
    /// these values via the `SyncState` checkpoint records the runtime
    /// emits at every commit boundary.
    pub fn sync_state_hash(&self) -> u64 {
        let mut hasher = cellgov_mem::Fnv1aHasher::new();
        for source in [
            self.mailbox_registry.state_hash(),
            self.signal_registry.state_hash(),
            self.lv2_host.state_hash(),
            self.syscall_responses.state_hash(),
        ] {
            hasher.write(&source.to_le_bytes());
        }
        hasher.finish()
    }

    /// Borrow committed guest memory.
    #[inline]
    pub fn memory(&self) -> &GuestMemory {
        &self.memory
    }

    /// Consume the runtime and return its guest memory.
    ///
    /// Used to chain execution phases: run module_start in one runtime,
    /// extract the memory (now containing initialized state), and create
    /// a new runtime for the game's entry point.
    pub fn into_memory(self) -> GuestMemory {
        self.memory
    }

    /// Current guest time. Advances by `result.consumed_budget` after
    /// each successful step.
    #[inline]
    pub fn time(&self) -> GuestTicks {
        self.time
    }

    /// Current epoch. Currently only advances at commit boundaries,
    /// which the commit pipeline owns; `step()` never advances it.
    #[inline]
    pub fn epoch(&self) -> Epoch {
        self.epoch
    }

    /// Total number of successful `step()` calls so far.
    #[inline]
    pub fn steps_taken(&self) -> usize {
        self.steps_taken
    }

    /// Configured max-steps cap.
    #[inline]
    pub fn max_steps(&self) -> usize {
        self.max_steps
    }

    /// Drain all pending DMA completions regardless of their scheduled
    /// time, applying each transfer to committed memory. Used at
    /// scenario termination to ensure all in-flight transfers become
    /// visible in the final memory snapshot.
    pub fn drain_pending_dma(&mut self) {
        let due = self.dma_queue.pop_due(GuestTicks::new(u64::MAX));
        for (c, payload) in &due {
            let bytes = if let Some(data) = payload {
                data.clone()
            } else if let Some(src) = self.memory.read(c.source()) {
                src.to_vec()
            } else {
                continue;
            };
            let _ = self.memory.apply_commit(c.destination(), &bytes);
        }
    }

    /// Drive one pipeline pass: select a unit, grant budget, run it,
    /// advance guest time.
    ///
    /// Returns `Err(StepError::MaxStepsExceeded)` if the deadlock
    /// detector tripped, `Err(StepError::NoRunnableUnit)` if the
    /// scheduler found nothing to run, and `Err(StepError::TimeOverflow)`
    /// if the consumed budget would push guest time past `u64::MAX`.
    /// On success, returns a [`RuntimeStep`] describing what happened.
    ///
    /// The unit's emitted effects are returned verbatim in
    /// [`RuntimeStep::result`]; this method does not validate, stage,
    /// commit, or otherwise act on them. The commit pipeline is a
    /// separate concern that wraps `step()` in a future slice.
    pub fn step(&mut self) -> Result<RuntimeStep, StepError> {
        if self.steps_taken >= self.max_steps {
            return Err(StepError::MaxStepsExceeded);
        }

        let unit_id = self
            .scheduler
            .select_next(&self.registry)
            .ok_or(StepError::NoRunnableUnit)?;

        // The unit is about to run. Clear any runtime-side status
        // override so the unit's own status logic resumes control
        // after this step.
        self.registry.clear_status_override(unit_id);

        // Trace pipeline step 1+2: scheduler selected `unit_id` and
        // granted `budget_per_step`. Per-step records only in FullTrace.
        if self.mode == RuntimeMode::FullTrace {
            self.trace.record(&TraceRecord::UnitScheduled {
                unit: unit_id,
                granted_budget: self.budget_per_step,
                time: self.time,
                epoch: self.epoch,
            });
        }

        // Build the readonly memory view for this step. The borrow
        // is alive only for the duration of run_until_yield, which is
        // exactly the freeze-during-step rule.
        // Drain any messages or syscall returns the commit pipeline
        // delivered to this unit and pass them via ExecutionContext.
        let received = self.registry.drain_receives(unit_id);
        let syscall_ret = self.registry.drain_syscall_return(unit_id);
        let reg_writes = self.registry.drain_register_writes(unit_id);
        let result = {
            let ctx = if let Some(code) = syscall_ret {
                if reg_writes.is_empty() {
                    ExecutionContext::with_syscall_return(&self.memory, &received, code)
                } else {
                    ExecutionContext::with_syscall_return_and_regs(
                        &self.memory,
                        &received,
                        code,
                        &reg_writes,
                    )
                }
            } else {
                ExecutionContext::with_received(&self.memory, &received)
            };
            let unit = self
                .registry
                .get_mut(unit_id)
                .expect("scheduler returned an id that is not in the registry");
            let res = unit.run_until_yield(self.budget_per_step, &ctx);
            // Drain per-instruction state fingerprints and full snapshots
            // collected during the step. Both empty unless the unit
            // has the corresponding mode on; drain is stable across
            // every unit via the trait defaults.
            let retired_hashes = unit.drain_retired_state_hashes();
            let retired_full = unit.drain_retired_state_full();
            (res, retired_hashes, retired_full)
        };
        let (result, retired_hashes, retired_full) = result;

        // Per-step divergence trace: one TraceRecord::PpuStateHash
        // per retired instruction in the main trace stream. Step indices
        // are assigned monotonically, independent of `steps_taken`.
        // Then one TraceRecord::PpuStateFull per windowed snapshot in
        // the separate zoom-in stream. Both are paired by step index
        // so the diff printer can match a hash divergence with its
        // full-state snapshot when both are present.
        let hash_base = self.per_step_index;
        for (pc, hash) in retired_hashes {
            self.trace.record(&TraceRecord::PpuStateHash {
                step: self.per_step_index,
                pc,
                hash: cellgov_trace::StateHash::new(hash),
            });
            self.per_step_index += 1;
        }
        // Each full-state snapshot is paired with the per-step hash at
        // the same retired index. We assume the unit emits full states
        // for a contiguous prefix of the same retirement sequence the
        // hashes use. The window's start index is unit-relative; here
        // we only know how many full-state entries the unit drained,
        // so we stamp them with `hash_base + i` -- correct only when
        // the window starts at the unit's retirement_counter == 0.
        // Larger windows that start mid-run get correct PCs but do not
        // attempt to recover the original step index (the diff printer
        // matches by PC instead).
        for (i, (pc, gpr, lr, ctr, xer, cr)) in retired_full.into_iter().enumerate() {
            self.zoom_trace.record(&TraceRecord::PpuStateFull {
                step: hash_base + i as u64,
                pc,
                gpr,
                lr,
                ctr,
                xer,
                cr,
            });
        }

        // Identity time-advancement policy: budget units convert 1:1
        // to guest ticks. The time-advancement policy can be any
        // deterministic function; identity is the
        // simplest such policy and is locked behind this single line
        // so future replacements have one place to look.
        let advance = GuestTicks::new(result.consumed_budget.raw());
        let time_after = self
            .time
            .checked_add(advance)
            .ok_or(StepError::TimeOverflow)?;
        self.time = time_after;
        self.steps_taken += 1;
        self.last_scheduled_unit = Some(unit_id);

        // Trace pipeline step 3+9: per-step records only in FullTrace.
        if self.mode == RuntimeMode::FullTrace {
            self.trace.record(&TraceRecord::StepCompleted {
                unit: unit_id,
                yield_reason: traced_yield_reason(result.yield_reason),
                consumed_budget: result.consumed_budget,
                time_after,
            });

            // Trace pipeline step 4: one EffectEmitted per effect.
            for (sequence, effect) in result.emitted_effects.iter().enumerate() {
                self.trace.record(&TraceRecord::EffectEmitted {
                    unit: unit_id,
                    sequence: sequence as u32,
                    kind: traced_effect_kind(effect),
                });
            }
        }

        Ok(RuntimeStep {
            unit: unit_id,
            result,
            time_after,
            epoch_after: self.epoch,
        })
    }
}

/// Map an `Effect` onto its `TracedEffectKind` twin.
///
/// Same DAG situation as `traced_yield_reason`: `cellgov_trace` sits
/// below `cellgov_effects` in the workspace and cannot import the
/// source enum, so the bridge is an exhaustive match. Adding a new
/// `Effect` variant breaks compilation here and forces the trace
/// contract to update at the same time.
fn traced_effect_kind(e: &cellgov_effects::Effect) -> TracedEffectKind {
    use cellgov_effects::Effect;
    match e {
        Effect::SharedWriteIntent { .. } => TracedEffectKind::SharedWriteIntent,
        Effect::MailboxSend { .. } => TracedEffectKind::MailboxSend,
        Effect::MailboxReceiveAttempt { .. } => TracedEffectKind::MailboxReceiveAttempt,
        Effect::DmaEnqueue { .. } => TracedEffectKind::DmaEnqueue,
        Effect::WaitOnEvent { .. } => TracedEffectKind::WaitOnEvent,
        Effect::WakeUnit { .. } => TracedEffectKind::WakeUnit,
        Effect::SignalUpdate { .. } => TracedEffectKind::SignalUpdate,
        Effect::FaultRaised { .. } => TracedEffectKind::FaultRaised,
        Effect::TraceMarker { .. } => TracedEffectKind::TraceMarker,
    }
}

/// Map a runtime [`YieldReason`] onto its [`TracedYieldReason`] twin.
///
/// The two enums live in different crates (`cellgov_exec` and
/// `cellgov_trace`) because the trace crate sits below `cellgov_exec`
/// in the workspace DAG and cannot depend on it. Their discriminants
/// are intentionally identical, but Rust does not let us cast between
/// distinct enum types directly, so we route through this exhaustive
/// match. The exhaustiveness is the load-bearing piece: if a new
/// `YieldReason` variant is ever added, this match stops compiling and
/// forces the trace contract to be updated in lockstep.
fn traced_yield_reason(y: YieldReason) -> TracedYieldReason {
    match y {
        YieldReason::BudgetExhausted => TracedYieldReason::BudgetExhausted,
        YieldReason::MailboxAccess => TracedYieldReason::MailboxAccess,
        YieldReason::DmaSubmitted => TracedYieldReason::DmaSubmitted,
        YieldReason::DmaWait => TracedYieldReason::DmaWait,
        YieldReason::WaitingSync => TracedYieldReason::WaitingSync,
        YieldReason::Syscall => TracedYieldReason::Syscall,
        YieldReason::InterruptBoundary => TracedYieldReason::InterruptBoundary,
        YieldReason::Fault => TracedYieldReason::Fault,
        YieldReason::Finished => TracedYieldReason::Finished,
    }
}

struct MemoryView<'a>(&'a GuestMemory);

impl Lv2Runtime for MemoryView<'_> {
    fn read_committed(&self, addr: u64, len: usize) -> Option<&[u8]> {
        let bytes = self.0.as_bytes();
        let start = addr as usize;
        let end = start.checked_add(len)?;
        if end <= bytes.len() {
            Some(&bytes[start..end])
        } else {
            None
        }
    }
}

#[cfg(test)]
#[path = "tests/runtime_tests.rs"]
mod tests;
