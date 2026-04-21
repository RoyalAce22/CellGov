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
//! - [`TraceRecord::PpuStateHash`] for each retired instruction a unit
//!   reports via `drain_retired_state_hashes`, stamped with a
//!   monotonic step index and carrying the post-retirement PC.
//! - [`TraceRecord::PpuStateFull`] for each full-register snapshot a
//!   unit reports via `drain_retired_state_full`, emitted into the
//!   separate zoom-in trace stream so the main trace stays
//!   fixed-size-per-record.
//!
//! The trace format is binary, not text. Tests pull
//! the bytes out via [`Runtime::trace`] and feed them to the
//! [`cellgov_trace::TraceReader`] for assertions. Text rendering is a
//! downstream tool over the same binary buffer.

use crate::commit::{CommitContext, CommitError, CommitOutcome, CommitPipeline};
use crate::registry::{RegisteredUnit, UnitRegistry};
use crate::scheduler::Scheduler;
use crate::syscall_table::SyscallResponseTable;
use cellgov_dma::{DmaLatencyModel, DmaQueue};
use cellgov_effects::Effect;
use cellgov_event::UnitId;
use cellgov_exec::{ExecutionContext, ExecutionStepResult, YieldReason};
use cellgov_lv2::{Lv2Host, PpuThreadInitState, SpuInitState};
use cellgov_mem::GuestMemory;
use cellgov_sync::{MailboxRegistry, SignalRegistry};
use cellgov_time::{Budget, Epoch, GuestTicks};
use cellgov_trace::{TraceRecord, TraceWriter};

mod commit_trace;
mod construction;
mod dma;
mod lv2_dispatch;
mod mem_helpers;
mod ppu_create;
mod sync_wakes;
mod trace_bridge;
mod types;
pub use types::{
    default_budget_for_mode, PpuFactory, RuntimeMode, RuntimeStep, SpuFactory, StepError,
};

use trace_bridge::{traced_effect_kind, traced_yield_reason};

/// The top-level runtime.
///
/// Composes the registry, scheduler, committed memory, guest time,
/// and epoch into a single object that drives one step at a time via
/// [`Runtime::step`]. The scheduler is pluggable via
/// [`Runtime::set_scheduler`]; defaults to [`crate::RoundRobinScheduler`].
///
/// Deterministic step-loop runtime over guest memory and registered units.
pub struct Runtime {
    pub(crate) registry: UnitRegistry,
    mailbox_registry: MailboxRegistry,
    signal_registry: SignalRegistry,
    reservations: cellgov_sync::ReservationTable,
    /// RSX FIFO cursor tracking put / get / current_reference. Folds
    /// into [`Runtime::sync_state_hash`] at every commit boundary so a
    /// cursor change is visible in the per-step state-hash trace. The
    /// FIFO advance pass (wired in a later slice) is the sole mutator
    /// of `get`; `put` is written via the RSX IO region pathway; the
    /// savestate-restore path may overwrite all three.
    rsx_cursor: crate::rsx::RsxFifoCursor,
    /// RSX semaphore offset register. Set by the
    /// `NV406E_SEMAPHORE_OFFSET` handler and consumed by the next
    /// `NV406E_SEMAPHORE_RELEASE`. Persists across commit boundaries
    /// because a guest that splits the OFFSET / RELEASE pair across
    /// FIFO drains must still read the offset the first drain wrote.
    /// Folded into [`Runtime::sync_state_hash`] so a leaked value
    /// surfaces at a state-hash diff rather than as a silent cross-
    /// drain carryover.
    rsx_sem_offset: u32,
    /// When true, writes that commit successfully to
    /// `0xC000_0040..0xC000_004C` (the PS3 RSX control register's
    /// put / get / reference slots) are mirrored into
    /// [`Self::rsx_cursor`] at the same commit boundary. When
    /// false, the cursor is only mutated by the FIFO advance pass
    /// and by direct [`Self::rsx_cursor_mut`] callers (tests /
    /// savestate restore). The mirror path is off by default --
    /// default boot runs keep the PS3 RSX region reserved so the
    /// first put-pointer write trips the FirstRsxWrite checkpoint;
    /// enabling this path only makes sense once the hosting layer
    /// has also made the RSX region writable.
    rsx_mirror_writes: bool,
    /// RSX flip-status state machine. Tracks the WAITING / DONE
    /// byte the guest polls after issuing `cellGcmSetFlip`, plus
    /// the flip-handler callback address (recorded only; the
    /// callback is not dispatched into PPU execution). Folds into
    /// [`Self::sync_state_hash`] so a flip transition is visible
    /// in the per-step state-hash trace.
    rsx_flip: crate::rsx_flip::RsxFlipState,
    /// NV method dispatch table for the RSX FIFO advance pass.
    /// Populated at construction with the registered handlers
    /// (NV406E semaphore / reference, NV4097 flip / report / back-
    /// end semaphore). The advance pass is the only reader.
    rsx_methods: crate::rsx_method::NvMethodTable,
    /// RSX effects produced by the FIFO advance pass at the END of
    /// commit batch N, queued for the START of batch N+1. Preserves
    /// the atomic-batch contract: FIFO method parses happen in
    /// batch N (they mutate the cursor + sem_offset committed
    /// state), but the downstream memory / state effects they
    /// produce commit alongside batch N+1's unit effects. A
    /// commit_step with no pending-and-no-new RSX effects pays zero
    /// extra cost.
    pending_rsx_effects: Vec<Effect>,
    dma_queue: DmaQueue,
    dma_latency: Box<dyn DmaLatencyModel>,
    lv2_host: Lv2Host,
    syscall_responses: SyscallResponseTable,
    spu_factory: Option<SpuFactory>,
    ppu_factory: Option<PpuFactory>,
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
    /// `commit_step()` to attribute the commit record, syscall
    /// dispatch, DMA issuer, and join-wake resolution to the unit that
    /// produced the batch -- there is one commit batch per unit
    /// yield, so this is always the right unit.
    last_scheduled_unit: Option<UnitId>,
    /// HLE-specific bookkeeping bundled off the main struct so
    /// `Runtime`'s field list reads as orchestration-only state.
    /// See [`crate::hle::HleState`].
    pub(crate) hle: crate::hle::HleState,
    /// Reusable effects buffer, taken/returned across steps to avoid
    /// per-step allocation in the common zero-effects case.
    effects_buf: Vec<Effect>,
    /// Controls trace and hash checkpoint overhead. Set via
    /// `Runtime::set_mode`; the constructor leaves this at
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
    /// Drive the commit pipeline for a previously-returned step result.
    ///
    /// Validates, stages, and applies the effects in `result` against
    /// committed memory, then performs the runtime-level work that
    /// surrounds the commit:
    ///
    /// - advances the epoch (on every commit boundary, including
    ///   validation failures, since the step's set of effects is
    ///   "closed" either way -- so an `Err` return still mutates
    ///   `self.epoch`);
    /// - dispatches `YieldReason::Syscall` through the LV2 host
    ///   (immediate return, SPU registration, or block);
    /// - pops DMA completions whose modeled time has arrived, applies
    ///   their transfers, and wakes their issuers;
    /// - resolves join wakes when an SPU finishes its group;
    /// - emits `CommitApplied`, `UnitBlocked`, and `UnitWoken` trace
    ///   records (outside `FaultDriven` mode);
    /// - writes the four `StateHashCheckpoint` records (committed
    ///   memory, runnable queue, unit status, sync state) outside
    ///   `FaultDriven` mode.
    ///
    /// Fault rule and atomic-batch semantics are inherited from
    /// [`CommitPipeline::process`].
    pub fn commit_step(
        &mut self,
        result: &ExecutionStepResult,
        effects: &[Effect],
    ) -> Result<CommitOutcome, CommitError> {
        // Trivial-step fast path: under FaultDriven mode, a step
        // that emitted no effects, did not fault, did not yield via
        // Syscall or Finished, and found the DMA queue empty has
        // no work for the commit pipeline. Epoch still advances
        // (atomic-batch boundary preserved); trace records are
        // already off under FaultDriven mode, so the observable
        // contract is identical. Cuts the per-step commit cost for
        // the PPU-bound hot loops that dominate game boots.
        if self.mode == RuntimeMode::FaultDriven
            && effects.is_empty()
            && result.fault.is_none()
            && !matches!(
                result.yield_reason,
                YieldReason::Syscall | YieldReason::Finished
            )
            && self.dma_queue.is_empty()
            && self.pending_rsx_effects.is_empty()
            && self.rsx_cursor.get() == self.rsx_cursor.put()
            && !self.rsx_flip.pending()
        {
            self.epoch = self.epoch.next().expect("epoch overflow");
            return Ok(CommitOutcome::default());
        }

        // Prepend any RSX effects the previous commit's advance pass
        // emitted into the current batch. Allocates only when
        // pending_rsx_effects is non-empty, which is rare before the
        // first put-pointer advance and common after.
        let combined_storage: Vec<Effect>;
        let effects: &[Effect] = if self.pending_rsx_effects.is_empty() {
            effects
        } else {
            combined_storage = self
                .pending_rsx_effects
                .drain(..)
                .chain(effects.iter().cloned())
                .collect();
            &combined_storage
        };

        // Snapshot flip.pending at commit-step entry so the post-
        // apply DONE transition fires only for flips that started
        // pending in a PRIOR batch -- not ones that just became
        // pending via a RsxFlipRequest applied in this batch.
        // Preserves the contract that an intermediate WAITING
        // observation is guaranteed for any PPU step between the
        // two commit boundaries.
        let flip_pending_at_entry = self.rsx_flip.pending();
        let flip_status_at_entry = self.rsx_flip.status();

        let mut ctx = CommitContext {
            memory: &mut self.memory,
            units: &mut self.registry,
            mailboxes: &mut self.mailbox_registry,
            signals: &mut self.signal_registry,
            dma_queue: &mut self.dma_queue,
            dma_latency: self.dma_latency.as_ref(),
            now: self.time,
            reservations: &mut self.reservations,
            rsx_label_base: self.hle.gcm.label_addr,
            rsx_flip: &mut self.rsx_flip,
        };
        let mut outcome = self.commit_pipeline.process(result, effects, &mut ctx);

        // Notify units that cached decoded instructions about any
        // committed code writes so their predecoded shadow can
        // mark the affected slots stale. Only fires when effects
        // contain SharedWriteIntent (stores), which is rare on
        // normal game hot loops (code and data segments are
        // separated by PT_LOAD boundaries) but required for
        // correctness whenever a title does self-modifying code
        // or runtime relocations after initial load.
        if outcome.is_ok() {
            for effect in effects {
                if let cellgov_effects::Effect::SharedWriteIntent { range, .. } = effect {
                    for (_, unit) in self.registry.iter_mut() {
                        unit.invalidate_code(range.start().raw(), range.length());
                    }
                }
            }
        }

        // RSX control-register writeback mirror. When
        // `rsx_mirror_writes` is enabled and the commit succeeded,
        // any committed write whose range overlaps the PS3 RSX
        // control register's put / get / reference slots
        // (0xC000_0040..0xC000_004C) is mirrored into the cursor so
        // the next FIFO advance pass sees the guest's new put (etc.)
        // without requiring an explicit runtime-side set_put call.
        // Reads bytes back from committed memory rather than the
        // effect payload because a partial-overlap write is legal
        // (e.g., a 4-byte store that straddles two slots) and the
        // committed bytes are the authoritative guest-visible
        // value. The mirror is a no-op on default boots (flag
        // defaults to false) -- the RSX region is reserved there,
        // so the write would fault rather than commit.
        if self.rsx_mirror_writes && outcome.is_ok() {
            self.mirror_rsx_control_register_writes(effects);
        }

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

        // RSX FIFO advance pass. Runs after unit effects have
        // committed, after DMA completions have fired, before state-
        // hash checkpoints are emitted. If the guest's effects
        // advanced put, drain the FIFO now -- emitted effects land
        // in self.pending_rsx_effects and commit alongside the next
        // batch's unit effects (atomic-batch contract). The cursor
        // mutations are visible in THIS batch's state-hash
        // checkpoint because the pass runs before emit_commit_trace.
        if self.rsx_cursor.get() != self.rsx_cursor.put() {
            crate::rsx_advance::rsx_advance(
                &self.memory,
                &mut self.rsx_cursor,
                &mut self.rsx_sem_offset,
                &self.rsx_methods,
                &mut self.pending_rsx_effects,
                self.time,
            );
        }

        // Flip-status WAITING -> DONE transition. Fires ONLY if
        // the flip was pending when this commit started (i.e., the
        // RsxFlipRequest landed in a PRIOR batch). This gives the
        // guest at least one full PPU step window in which a poll
        // returns WAITING before the next commit boundary flips it
        // to DONE. A new RsxFlipRequest applied in this batch sets
        // pending=true but will not transition until the NEXT
        // commit -- the one-batch-delay tightness contract.
        if flip_pending_at_entry {
            self.rsx_flip.complete_pending_flip();
        }

        // Flip-status memory mirror. If the flip status changed
        // (RsxFlipRequest applied or the DONE transition fired),
        // write the new byte-value as a 4-byte BE u32 at the
        // fixed mirror address so a guest polling through normal
        // loads observes it. No write on no-change -- keeps the
        // memory hash stable across no-flip commits. Writes land
        // in the RSX region, which is ReadWrite when rsx_mirror
        // is enabled; under the default reserved-region layout
        // this would fault, so gate on rsx_mirror.
        if self.rsx_mirror_writes {
            let flip_status_now = self.rsx_flip.status();
            if flip_status_now != flip_status_at_entry {
                let addr = crate::rsx::RSX_FLIP_STATUS_MIRROR_ADDR as u64;
                if let Some(range) =
                    cellgov_mem::ByteRange::new(cellgov_mem::GuestAddr::new(addr), 4)
                {
                    let value = flip_status_now as u32;
                    let _ = self.memory.apply_commit(range, &value.to_be_bytes());
                }
            }
        }

        // Attribution: there is one commit batch per unit yield, so
        // `source` (defined above) is the unit that produced the batch.
        // Commit-level trace records and state-hash checkpoints fire
        // in DeterminismCheck and FullTrace modes but not FaultDriven.
        self.emit_commit_trace(source, &outcome, &due);

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

    /// Borrow the zoom-in trace writer. Empty unless a unit had a
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
    /// `SyncState` hash checkpoint. The commit pipeline routes
    /// `MailboxSend` and `MailboxReceiveAttempt` effects through this
    /// registry.
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

    /// Install a PPU factory. The runtime calls this factory when
    /// `Lv2Dispatch::PpuThreadCreate` fires during `commit_step`
    /// to construct child PPU execution units seeded with the
    /// proper PPC64 ABI state. The CLI installs one at boot;
    /// tests that exercise thread creation install their own.
    pub fn set_ppu_factory<F>(&mut self, factory: F)
    where
        F: Fn(UnitId, PpuThreadInitState) -> Box<dyn RegisteredUnit> + 'static,
    {
        self.ppu_factory = Some(Box::new(factory));
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
        self.hle.nids = nids;
    }

    /// Set the base address for the HLE bump allocator (_sys_malloc).
    /// Also records the base in `HleState` so the watermark band
    /// check in `heap_alloc` computes "bytes handed out" against the
    /// correct origin, and resets the band-warning bitmask so a
    /// reconfigured run starts with a clean slate.
    pub fn set_hle_heap_base(&mut self, base: u32) {
        assert_ne!(
            base, 0,
            "set_hle_heap_base: heap_base = 0 would let heap_alloc hand out address 0, \
             which the dispatch witnesses in hle::cell_gcm_sys rely on being impossible"
        );
        self.hle.heap_base = base;
        self.hle.heap_ptr = base;
        self.hle.heap_watermark = base;
        self.hle.heap_warning_mask = 0;
    }

    /// Peak address the HLE bump allocator has ever reached. Subtract
    /// the heap base (the value most recently passed to
    /// [`Runtime::set_hle_heap_base`]) to get cumulative bytes
    /// allocated across the scenario. Diagnostic for sizing the
    /// HLE arena and deciding whether the bump-on-free policy
    /// needs to become a real free-list allocator. See the TODO
    /// on `NID_SYS_FREE` in `hle::sys_prx_for_user` for the design sketch.
    #[inline]
    pub fn hle_heap_watermark(&self) -> u32 {
        self.hle.heap_watermark
    }

    /// Map of NIDs the dispatcher has seen that no HLE module
    /// claimed, with per-NID call counts. Populated by the
    /// internal HLE dispatch path. Empty after a run means every
    /// imported library function was at least recognized; a
    /// non-empty map is a punch list of unimplemented PS3
    /// library entries the scenario actually touched.
    #[inline]
    pub fn hle_unclaimed_nids(&self) -> &std::collections::BTreeMap<u32, usize> {
        &self.hle.unclaimed_nids
    }

    /// Map of NIDs whose handlers ran but produced no observable
    /// mutation (no set_return, set_register, write_guest,
    /// set_unit_finished, heap_alloc, or alloc_id call) before the
    /// adapter dropped. Populated from the Drop impl of
    /// `RuntimeHleAdapter`. Same shape as
    /// [`Self::hle_unclaimed_nids`] but different population:
    /// these NIDs were *routed into a handler* that did nothing
    /// observable, which leaks stale register state through to the
    /// guest. A non-empty map in a production run is a
    /// silent-divergence punch list.
    #[inline]
    pub fn hle_handlers_without_mutation(&self) -> &std::collections::BTreeMap<u32, usize> {
        &self.hle.handlers_without_mutation
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

    /// Change the per-step budget grant. The new budget takes
    /// effect on the next `step()` call. Callers that want the
    /// natural budget for a mode should use
    /// [`default_budget_for_mode`] instead of picking a number.
    pub fn set_budget(&mut self, budget: Budget) {
        self.budget_per_step = budget;
    }

    /// Per-step budget the runtime currently grants units.
    pub fn budget(&self) -> Budget {
        self.budget_per_step
    }

    /// Combined hash of every sync / committed-state source the
    /// runtime owns: mailbox queues, signal-notification registers,
    /// LV2 host state, syscall responses, reservation table, and the
    /// RSX FIFO cursor. Computed by FNV-1a-merging the per-source
    /// hashes in a fixed source order so the result is deterministic
    /// and stable across runs of the same scenario. Replay tooling
    /// compares pairs of these values via the `SyncState` checkpoint
    /// records the runtime emits at every commit boundary.
    pub fn sync_state_hash(&self) -> u64 {
        let mut hasher = cellgov_mem::Fnv1aHasher::new();
        for source in [
            self.mailbox_registry.state_hash(),
            self.signal_registry.state_hash(),
            self.lv2_host.state_hash(),
            self.syscall_responses.state_hash(),
            self.reservations.state_hash(),
            self.rsx_cursor.state_hash(),
            self.rsx_sem_offset as u64,
            self.rsx_flip.state_hash(),
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

    /// When set, _cellGcmInitBody places the control register in the
    /// RSX reserved region so the game's first put-pointer write
    /// triggers a ReservedWrite commit error, which the CLI translates
    /// to the FirstRsxWrite checkpoint.
    pub fn set_gcm_rsx_checkpoint(&mut self, enabled: bool) {
        self.hle.gcm.rsx_checkpoint = enabled;
    }

    /// Mutable borrow of committed guest memory. Used by test
    /// infrastructure and CLI tooling that needs to patch guest
    /// memory between steps (e.g. self-modifying-code invalidation
    /// tests, or seeding TLS / PRX images mid-boot).
    #[inline]
    pub fn memory_mut(&mut self) -> &mut GuestMemory {
        &mut self.memory
    }

    /// Borrow the committed atomic-reservation table. Primarily
    /// for tests that assert on reservation-sweep behavior.
    #[inline]
    pub fn reservations(&self) -> &cellgov_sync::ReservationTable {
        &self.reservations
    }

    /// Mutable borrow of the reservation table. Used by tests
    /// that need to pre-populate entries before driving a write
    /// through the commit pipeline.
    #[inline]
    pub fn reservations_mut(&mut self) -> &mut cellgov_sync::ReservationTable {
        &mut self.reservations
    }

    /// Borrow the RSX FIFO cursor. Exposed so tests and the CLI can
    /// assert on put / get / current_reference without round-tripping
    /// through the sync state hash.
    #[inline]
    pub fn rsx_cursor(&self) -> &crate::rsx::RsxFifoCursor {
        &self.rsx_cursor
    }

    /// Mutable borrow of the RSX FIFO cursor. Exposed for the
    /// method-advance pass (wired in a later slice), the savestate-
    /// restore path, and tests that script cursor mutations.
    #[inline]
    pub fn rsx_cursor_mut(&mut self) -> &mut crate::rsx::RsxFifoCursor {
        &mut self.rsx_cursor
    }

    /// Current value of the RSX semaphore-offset register. Tests
    /// and the advance pass use this to assert cross-drain
    /// persistence of the offset.
    #[inline]
    pub fn rsx_sem_offset(&self) -> u32 {
        self.rsx_sem_offset
    }

    /// Mutable borrow of the RSX semaphore-offset register. Exposed
    /// for the method-advance pass (wired in a later slice) and for
    /// tests that script the offset directly.
    #[inline]
    pub fn rsx_sem_offset_mut(&mut self) -> &mut u32 {
        &mut self.rsx_sem_offset
    }

    /// Enable (or disable) the RSX control-register writeback
    /// mirror. When enabled, PPU writes to the control-register
    /// window (`0xC0000040` / `0xC0000044`) mirror into the
    /// runtime's `RsxFifoCursor` at the same commit boundary. The
    /// host is responsible for ensuring the PS3 RSX region is
    /// writable before enabling this; flipping it to `true` against
    /// a reserved region means every put-pointer store still
    /// reserved-writes and the mirror never runs.
    pub fn set_rsx_mirror_writes(&mut self, enabled: bool) {
        self.rsx_mirror_writes = enabled;
    }

    /// Current value of the RSX mirror flag. Primarily for
    /// test-side assertions and CLI status reporting.
    #[inline]
    pub fn rsx_mirror_writes_enabled(&self) -> bool {
        self.rsx_mirror_writes
    }

    /// Borrow the RSX flip-status state. Tests and the CLI use
    /// this to observe the WAITING / DONE transition and the
    /// registered handler address.
    #[inline]
    pub fn rsx_flip(&self) -> &crate::rsx_flip::RsxFlipState {
        &self.rsx_flip
    }

    /// Mutable borrow of the RSX flip-status state. Exposed for
    /// the `NV4097_FLIP_BUFFER` handler, the `cellGcmSetFlipHandler`
    /// HLE NID, the per-boundary DONE transition, and tests that
    /// script the state directly.
    #[inline]
    pub fn rsx_flip_mut(&mut self) -> &mut crate::rsx_flip::RsxFlipState {
        &mut self.rsx_flip
    }

    /// Mirror any committed write to the PS3 RSX control register's
    /// put / get / reference slots into [`Self::rsx_cursor`].
    ///
    /// Walks `effects` looking for `SharedWriteIntent`s whose range
    /// overlaps `0xC000_0040..0xC000_004C`. For each overlap, reads
    /// the committed bytes out of guest memory (not out of the
    /// effect payload, because a partial-overlap write may cross
    /// two slots and the authoritative value is the one the commit
    /// pipeline just applied) and updates the corresponding cursor
    /// field. Each slot is only mirrored when its FULL 4-byte
    /// window was touched by the write -- a sub-word store is
    /// still applied to memory but does not update the cursor
    /// (real PS3 guest code writes all 4 bytes at once; any sub-
    /// word store is either a bug or an alignment test, and
    /// fabricating a cursor value from it would hide the guest bug).
    ///
    /// Called from `commit_step` after the commit pipeline has
    /// applied the batch and before the FIFO advance pass runs,
    /// so the FIFO drain sees the new put / ref in the same batch.
    fn mirror_rsx_control_register_writes(&mut self, effects: &[Effect]) {
        use crate::rsx::{RSX_CONTROL_GET_ADDR, RSX_CONTROL_PUT_ADDR, RSX_CONTROL_REF_ADDR};
        enum Slot {
            Put,
            Get,
            Ref,
        }
        const SLOTS: [(u32, Slot); 3] = [
            (RSX_CONTROL_PUT_ADDR, Slot::Put),
            (RSX_CONTROL_GET_ADDR, Slot::Get),
            (RSX_CONTROL_REF_ADDR, Slot::Ref),
        ];
        for effect in effects {
            let Effect::SharedWriteIntent { range, .. } = effect else {
                continue;
            };
            let write_start = range.start().raw();
            let write_end = write_start.saturating_add(range.length());
            for (slot_addr, slot) in SLOTS.iter() {
                let slot_start = *slot_addr as u64;
                let slot_end = slot_start + 4;
                if write_start <= slot_start && write_end >= slot_end {
                    // Full-word coverage: read the committed bytes
                    // out of guest memory and mirror.
                    let Some(slot_range) =
                        cellgov_mem::ByteRange::new(cellgov_mem::GuestAddr::new(slot_start), 4)
                    else {
                        continue;
                    };
                    if let Some(bytes) = self.memory.read(slot_range) {
                        // PS3 PPU is big-endian; the guest wrote
                        // a u32 via a big-endian store. Interpret
                        // bytes accordingly.
                        let value = u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
                        match slot {
                            Slot::Put => self.rsx_cursor.set_put(value),
                            Slot::Get => self.rsx_cursor.set_get(value),
                            Slot::Ref => self.rsx_cursor.set_reference(value),
                        }
                    }
                }
            }
        }
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

    /// Current epoch. Advances only at commit boundaries, which the
    /// commit pipeline owns; `step()` never advances it.
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

    /// Drive one pipeline pass: select a unit, grant budget, run it,
    /// advance guest time.
    ///
    /// Returns `Err(StepError::MaxStepsExceeded)` if the deadlock
    /// detector tripped, `Err(StepError::NoRunnableUnit)` for a
    /// terminal stall (empty registry or every unit faulted /
    /// finished), `Err(StepError::AllBlocked)` if at least one
    /// unit is parked on a block condition but none are runnable,
    /// and `Err(StepError::TimeOverflow)` if the consumed budget
    /// would push guest time past `u64::MAX`. On success, returns a
    /// [`RuntimeStep`] describing what happened.
    ///
    /// The unit's emitted effects are returned verbatim in
    /// [`RuntimeStep::result`]; this method does not validate, stage,
    /// commit, or otherwise act on them. [`Runtime::commit_step`] drives
    /// the commit pipeline over the returned effects.
    pub fn step(&mut self) -> Result<RuntimeStep, StepError> {
        if self.steps_taken >= self.max_steps {
            return Err(StepError::MaxStepsExceeded);
        }

        let unit_id = match self.scheduler.select_next(&self.registry) {
            Some(id) => id,
            None => {
                // Distinguish "terminal stall" (nothing left to run
                // or wake) from "all blocked" (parked units that
                // could be woken by a future signal). Check
                // effective status for each registered unit; if any
                // is Blocked we're in the soft-stall case.
                let any_blocked = self.registry.ids().any(|id| {
                    self.registry.effective_status(id) == Some(cellgov_exec::UnitStatus::Blocked)
                });
                return Err(if any_blocked {
                    StepError::AllBlocked
                } else {
                    StepError::NoRunnableUnit
                });
            }
        };

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
        let mut effects_buf = std::mem::take(&mut self.effects_buf);
        effects_buf.clear();
        let (result, retired_hashes, retired_full) = {
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
            let ctx = ctx.with_reservations(&self.reservations);
            let unit = self
                .registry
                .get_mut(unit_id)
                .expect("scheduler returned an id that is not in the registry");
            let res = unit.run_until_yield(self.budget_per_step, &ctx, &mut effects_buf);
            // Drain per-instruction state fingerprints and full snapshots
            // collected during the step. Both empty unless the unit
            // has the corresponding mode on; drain is stable across
            // every unit via the trait defaults. Under FaultDriven
            // mode no downstream consumer reads the drained data, so
            // skip both vtable dispatches entirely -- the per-step
            // saving shows up in PPU-bound hot loops that otherwise
            // see two trait-call round trips for nothing.
            let (retired_hashes, retired_full) = if self.mode == RuntimeMode::FaultDriven {
                (Vec::new(), Vec::new())
            } else {
                (
                    unit.drain_retired_state_hashes(),
                    unit.drain_retired_state_full(),
                )
            };
            (res, retired_hashes, retired_full)
        };

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
        // Full-state snapshots are stamped with `hash_base + i` so
        // that, for a zoom-in window starting at the unit's first
        // retired instruction, the `step` field of each `PpuStateFull`
        // equals the `step` field of the matching `PpuStateHash`.
        // Windows that start mid-run carry correct PCs but the `step`
        // field no longer aligns with the hash stream; the diff
        // printer matches by PC in that case.
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
        // to guest ticks. Any deterministic function of consumed
        // budget satisfies the determinism contract; the identity
        // mapping is the one this runtime ships.
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
            for (sequence, effect) in effects_buf.iter().enumerate() {
                self.trace.record(&TraceRecord::EffectEmitted {
                    unit: unit_id,
                    sequence: sequence as u32,
                    kind: traced_effect_kind(effect),
                });
            }
        }

        // Move effects_buf into RuntimeStep; self.effects_buf starts
        // fresh next step. In FaultDriven mode (most steps have 0
        // effects), effects_buf is empty, so the fresh Vec does not
        // allocate.
        self.effects_buf = Vec::new();
        Ok(RuntimeStep {
            unit: unit_id,
            result,
            effects: effects_buf,
            time_after,
            epoch_after: self.epoch,
        })
    }
}

#[cfg(test)]
#[path = "tests/runtime_tests.rs"]
mod tests;
