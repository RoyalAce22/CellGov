//! The `Runtime` struct -- top-level orchestration.
//!
//! The runtime pipeline has ten steps. This module owns the
//! orchestration skeleton that owns the components every step needs
//! (registry, scheduler, memory, time, epoch) and composes pipeline
//! steps 1-3 and step 9 into a single [`Runtime::step`] call:
//!
//! 1. Select a runnable unit deterministically (via [`crate::Scheduler`])
//! 2. Grant the configured per-step budget
//! 3. Run the unit until it yields
//! 9. Advance guest time by the unit's consumed budget
//!
//! Steps 4-8 (collect, validate, stage, commit, inject events) are
//! handled at a higher layer. The [`Runtime::step`]
//! contract returns the unit's [`ExecutionStepResult`] verbatim;
//! callers (the future commit pipeline, today the tests) decide what
//! to do with the emitted effects. This is deliberate: the commit
//! pipeline is its own atomic concern and lands separately so that
//! both pieces can be reviewed independently.
//!
//! Step 10 (trace every decision) is also deferred -- the binary
//! trace writer does not yet exist. The runtime exposes enough state
//! that a trace layer can wrap [`Runtime::step`] later without
//! needing changes here.
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
//! The trace format is binary from day one. Tests pull
//! the bytes out via [`Runtime::trace`] and feed them to the
//! [`cellgov_trace::TraceReader`] for assertions. Text rendering is a
//! downstream tool over the same binary buffer.

use crate::commit::{CommitError, CommitOutcome, CommitPipeline};
use crate::registry::UnitRegistry;
use crate::scheduler::{RoundRobinScheduler, Scheduler};
use cellgov_dma::{DmaLatencyModel, DmaQueue, FixedLatency};
use cellgov_event::UnitId;
use cellgov_exec::{ExecutionContext, ExecutionStepResult, UnitStatus, YieldReason};
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
/// [`Runtime::step`]. Currently hardcodes the scheduler to
/// [`RoundRobinScheduler`]; a future refactor can take a
/// `Box<dyn Scheduler>` parameter without changing the public step
/// contract.
pub struct Runtime {
    registry: UnitRegistry,
    mailbox_registry: MailboxRegistry,
    signal_registry: SignalRegistry,
    dma_queue: DmaQueue,
    dma_latency: Box<dyn DmaLatencyModel>,
    scheduler: RoundRobinScheduler,
    commit_pipeline: CommitPipeline,
    memory: GuestMemory,
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
            scheduler: RoundRobinScheduler::new(),
            commit_pipeline: CommitPipeline::new(),
            memory,
            time: GuestTicks::ZERO,
            epoch: Epoch::ZERO,
            budget_per_step,
            steps_taken: 0,
            max_steps,
            trace,
            last_scheduled_unit: None,
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
        let mut outcome = self.commit_pipeline.process(
            result,
            &mut self.memory,
            &mut self.registry,
            &mut self.mailbox_registry,
            &mut self.signal_registry,
            &mut self.dma_queue,
            self.dma_latency.as_ref(),
            self.time,
        );
        // Epoch advances on every commit boundary, even on validation
        // failure. Overflow is a runtime invariant violation; the
        // surrounding deadlock detector should fire long before we
        // get anywhere near 2^64 commits.
        self.epoch = self.epoch.next().expect("epoch overflow");

        // Fire DMA completions whose modeled time has arrived. Each
        // fired completion applies its transfer to committed memory
        // and wakes the issuing unit so it can observe the result on
        // its next step. Runs on both Ok and Err outcome paths
        // because completions are from previous steps, not the
        // current one.
        let due = self.dma_queue.pop_due(self.time);
        for (c, payload) in &due {
            // Apply the transfer. If the effect carried an inline
            // payload (e.g., SPU local store bytes), use that directly.
            // Otherwise read the source bytes from committed guest
            // memory (the normal path for guest-memory-to-guest-memory
            // transfers).
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
        if let Ok(ref mut o) = outcome {
            o.dma_completions_fired = due.len();
        }

        // Trace pipeline step 7 (and the validation rejection edge): one
        // CommitApplied record per commit boundary, carrying the
        // post-commit epoch. On validation failure, rejection surfaces
        // as a fault on the originating unit, so
        // we record fault_discarded = true with zero counts -- the
        // batch is closed, just empty.
        //
        // Attribution: there is one commit batch per unit yield, so
        // the source is the unit selected by the most recent step()
        // call. If no step has run yet, fall back to UnitId::new(0) so
        // the trace remains a well-formed binary stream.
        let source = self.last_scheduled_unit.unwrap_or_else(|| UnitId::new(0));
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

        // Emit block/wake transition records. The commit pipeline
        // tracks which units were blocked (empty mailbox, wait-on-event)
        // and woken (wake effect). DMA completion wakes are emitted
        // separately since they are handled by the runtime, not the
        // pipeline.
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

        // State hash checkpoints. Four kinds:
        // committed memory, runnable queue, sync state, and unit
        // status. All four are emitted here, taken AFTER the commit
        // (including DMA completion firing) so replay tooling sees
        // post-commit state.
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

        outcome
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

    /// Borrow the DMA completion queue.
    #[inline]
    pub fn dma_queue(&self) -> &DmaQueue {
        &self.dma_queue
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
    /// queues plus signal-notification registers. Computed by
    /// FNV-1a-merging the per-source hashes in a fixed source order
    /// (mailboxes, then signals) so the result is deterministic and
    /// stable across runs of the same scenario. Replay tooling
    /// compares pairs of these values via the `SyncState` checkpoint
    /// records the runtime emits at every commit boundary.
    pub fn sync_state_hash(&self) -> u64 {
        const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
        const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;
        let mut h = FNV_OFFSET;
        for source in [
            self.mailbox_registry.state_hash(),
            self.signal_registry.state_hash(),
        ] {
            for b in source.to_le_bytes() {
                h ^= b as u64;
                h = h.wrapping_mul(FNV_PRIME);
            }
        }
        h
    }

    /// Borrow committed guest memory.
    #[inline]
    pub fn memory(&self) -> &GuestMemory {
        &self.memory
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
        // granted `budget_per_step`. Recorded *before* run_until_yield
        // so the trace makes the decision visible even if the unit
        // panics or runs forever (the deadlock detector is the next
        // safety net, but the schedule decision is already on record).
        self.trace.record(&TraceRecord::UnitScheduled {
            unit: unit_id,
            granted_budget: self.budget_per_step,
            time: self.time,
            epoch: self.epoch,
        });

        // Build the readonly memory view for this step. The borrow
        // is alive only for the duration of run_until_yield, which is
        // exactly the freeze-during-step rule.
        // Drain any messages the commit pipeline delivered to this
        // unit (e.g. from MailboxReceiveAttempt) and pass them via
        // ExecutionContext so the unit can read them during its step.
        let received = self.registry.drain_receives(unit_id);
        let result = {
            let ctx = ExecutionContext::with_received(&self.memory, &received);
            let unit = self
                .registry
                .get_mut(unit_id)
                .expect("scheduler returned an id that is not in the registry");
            unit.run_until_yield(self.budget_per_step, &ctx)
        };

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

        // Trace pipeline step 3+9: unit yielded with this reason after
        // consuming this much budget; guest time is now `time_after`.
        self.trace.record(&TraceRecord::StepCompleted {
            unit: unit_id,
            yield_reason: traced_yield_reason(result.yield_reason),
            consumed_budget: result.consumed_budget,
            time_after,
        });

        // Trace pipeline step 4: one EffectEmitted record per effect,
        // in emission order, with `sequence` running 0..N within the
        // step. Per-effect payloads (write bytes, mailbox messages,
        // DMA descriptors) are not in the trace yet; the kind alone is
        // enough for replay tooling to verify the effect sequence.
        for (sequence, effect) in result.emitted_effects.iter().enumerate() {
            self.trace.record(&TraceRecord::EffectEmitted {
                unit: unit_id,
                sequence: sequence as u32,
                kind: traced_effect_kind(effect),
            });
        }

        Ok(RuntimeStep {
            unit: unit_id,
            result,
            time_after,
            epoch_after: self.epoch,
        })
    }
}

/// Map an [`Effect`] onto its [`TracedEffectKind`] twin.
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

#[cfg(test)]
#[path = "tests/runtime_tests.rs"]
mod tests;
