//! Top-level runtime: [`Runtime::step`] selects a unit, grants budget, and
//! runs until yield; [`Runtime::commit_step`] validates, stages, and applies
//! the emitted effects, dispatches syscalls, fires due DMA completions,
//! and emits the commit-boundary trace records.
//!
//! Determinism contract: every observable output is a pure function of
//! runtime state + registry contents + scheduler decisions. No host time,
//! no `HashMap` iteration. The `max_steps` cap trips
//! [`StepError::MaxStepsExceeded`] rather than looping on a stalled system.
//!
//! The main trace stream is fixed-size-per-record; full PPU register
//! snapshots go to a separate `zoom_trace` writer so the main stream
//! stays homogeneous.

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

/// Deterministic step-loop runtime over guest memory and registered units.
pub struct Runtime {
    pub(crate) registry: UnitRegistry,
    mailbox_registry: MailboxRegistry,
    signal_registry: SignalRegistry,
    reservations: cellgov_sync::ReservationTable,
    /// Folded into [`Runtime::sync_state_hash`] at every commit boundary.
    rsx_cursor: crate::rsx::RsxFifoCursor,
    /// Persists across commit boundaries: an OFFSET / RELEASE pair may
    /// straddle drains and the later RELEASE must read the earlier OFFSET.
    /// Folded into [`Runtime::sync_state_hash`].
    rsx_sem_offset: u32,
    /// When true, committed writes to `0xC000_0040..0xC000_004C` mirror
    /// into [`Self::rsx_cursor`] at the same commit boundary. Host must
    /// make the RSX region writable before enabling; otherwise every
    /// put-pointer store reserved-writes and the mirror never runs.
    rsx_mirror_writes: bool,
    /// Folded into [`Self::sync_state_hash`].
    rsx_flip: crate::rsx::flip::RsxFlipState,
    rsx_methods: crate::rsx::method::NvMethodTable,
    /// Advance-pass effects produced at the end of commit batch N, queued
    /// for the start of batch N+1. Preserves the atomic-batch contract:
    /// FIFO method parses mutate cursor + sem_offset in batch N, but the
    /// downstream memory / state effects commit alongside batch N+1.
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
    /// Unit selected by the most recent `step()` call; one commit batch
    /// per unit yield, so this attributes the batch unambiguously.
    last_scheduled_unit: Option<UnitId>,
    pub(crate) hle: crate::hle::HleState,
    /// Reused across steps to avoid per-step allocation in the common
    /// zero-effects case.
    effects_buf: Vec<Effect>,
    mode: RuntimeMode,
    /// Monotonic counter over per-instruction state hashes. Orthogonal to
    /// `steps_taken`, which counts `run_until_yield` invocations.
    per_step_index: u64,
    /// Separate sink for `PpuStateFull` records so the main `trace` stream
    /// stays fixed-size-per-record.
    zoom_trace: TraceWriter,
}

impl Runtime {
    /// Drive the commit pipeline for a previously-returned step result.
    ///
    /// The epoch advances on every commit boundary, including validation
    /// failures -- the step's set of effects is closed either way, so an
    /// `Err` return still mutates `self.epoch`. Fault rule and atomic-batch
    /// semantics are inherited from [`CommitPipeline::process`].
    pub fn commit_step(
        &mut self,
        result: &ExecutionStepResult,
        effects: &[Effect],
    ) -> Result<CommitOutcome, CommitError> {
        // Trivial-step fast path under FaultDriven: no effects, no fault,
        // no syscall/finished yield, empty DMA and RSX queues. Epoch still
        // advances so the atomic-batch boundary is preserved; trace is
        // already off in this mode.
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
        // emitted. Allocates only when pending_rsx_effects is non-empty.
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

        // Snapshot at entry so the post-apply DONE transition fires only
        // for flips that started pending in a PRIOR batch; guarantees an
        // intermediate WAITING observation for any PPU step between the
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

        // Invalidate predecoded code cached by any unit that overlaps a
        // committed write. Required for correctness under self-modifying
        // code or runtime relocations.
        if outcome.is_ok() {
            for effect in effects {
                if let cellgov_effects::Effect::SharedWriteIntent { range, .. } = effect {
                    for (_, unit) in self.registry.iter_mut() {
                        unit.invalidate_code(range.start().raw(), range.length());
                    }
                }
            }
        }

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

        // RSX FIFO advance pass: runs after unit effects commit and DMA
        // completions fire, before state-hash checkpoints emit. Emitted
        // effects land in `pending_rsx_effects` and commit alongside the
        // next batch's unit effects (atomic-batch contract). Cursor
        // mutations land in THIS batch's state-hash checkpoint.
        if self.rsx_cursor.get() != self.rsx_cursor.put() {
            crate::rsx::advance::rsx_advance(
                &self.memory,
                &mut self.rsx_cursor,
                &mut self.rsx_sem_offset,
                &self.rsx_methods,
                &mut self.pending_rsx_effects,
                self.time,
            );
        }

        // WAITING -> DONE transition: only for flips pending at entry,
        // so a flip queued in THIS batch has at least one PPU step window
        // in which a poll returns WAITING before the next boundary.
        if flip_pending_at_entry {
            self.rsx_flip.complete_pending_flip();
        }

        // Flip-status memory mirror. Gated on rsx_mirror_writes because
        // the default reserved-region RSX layout would reserved-fault.
        // Skip on no-change to keep the memory hash stable.
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

        self.emit_commit_trace(source, &outcome, &due);

        outcome
    }

    /// Binary trace buffer accumulated so far.
    #[inline]
    pub fn trace(&self) -> &TraceWriter {
        &self.trace
    }

    /// Empty unless a unit had a zoom-in window configured.
    #[inline]
    pub fn zoom_trace(&self) -> &TraceWriter {
        &self.zoom_trace
    }

    /// Unit registry.
    #[inline]
    pub fn registry(&self) -> &UnitRegistry {
        &self.registry
    }

    /// Mutable unit registry.
    #[inline]
    pub fn registry_mut(&mut self) -> &mut UnitRegistry {
        &mut self.registry
    }

    /// Mailbox registry.
    #[inline]
    pub fn mailbox_registry(&self) -> &MailboxRegistry {
        &self.mailbox_registry
    }

    /// Mutable mailbox registry.
    #[inline]
    pub fn mailbox_registry_mut(&mut self) -> &mut MailboxRegistry {
        &mut self.mailbox_registry
    }

    /// Signal-notification register registry.
    #[inline]
    pub fn signal_registry(&self) -> &SignalRegistry {
        &self.signal_registry
    }

    /// Mutable signal-notification register registry.
    #[inline]
    pub fn signal_registry_mut(&mut self) -> &mut SignalRegistry {
        &mut self.signal_registry
    }

    /// LV2 host model.
    #[inline]
    pub fn lv2_host(&self) -> &Lv2Host {
        &self.lv2_host
    }

    /// Mutable LV2 host model.
    #[inline]
    pub fn lv2_host_mut(&mut self) -> &mut Lv2Host {
        &mut self.lv2_host
    }

    /// Invoked when `Lv2Dispatch::RegisterSpu` fires during `commit_step`.
    pub fn set_spu_factory<F>(&mut self, factory: F)
    where
        F: Fn(UnitId, SpuInitState) -> Box<dyn RegisteredUnit> + 'static,
    {
        self.spu_factory = Some(Box::new(factory));
    }

    /// Invoked when `Lv2Dispatch::PpuThreadCreate` fires during `commit_step`.
    pub fn set_ppu_factory<F>(&mut self, factory: F)
    where
        F: Fn(UnitId, PpuThreadInitState) -> Box<dyn RegisteredUnit> + 'static,
    {
        self.ppu_factory = Some(Box::new(factory));
    }

    /// Syscall response table.
    #[inline]
    pub fn syscall_responses(&self) -> &SyscallResponseTable {
        &self.syscall_responses
    }

    /// Mutable syscall response table.
    #[inline]
    pub fn syscall_responses_mut(&mut self) -> &mut SyscallResponseTable {
        &mut self.syscall_responses
    }

    /// DMA completion queue.
    #[inline]
    pub fn dma_queue(&self) -> &DmaQueue {
        &self.dma_queue
    }

    /// Replace the scheduler.
    pub fn set_scheduler<S: Scheduler + 'static>(&mut self, scheduler: S) {
        self.scheduler = Box::new(scheduler);
    }

    /// Map HLE index -> NID for dispatch.
    pub fn set_hle_nids(&mut self, nids: std::collections::BTreeMap<u32, u32>) {
        self.hle.nids = nids;
    }

    /// Reset the HLE bump allocator and its watermark-band warning mask.
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

    /// Peak address the HLE bump allocator has ever reached. Subtract the
    /// heap base to get cumulative bytes allocated across the scenario.
    #[inline]
    pub fn hle_heap_watermark(&self) -> u32 {
        self.hle.heap_watermark
    }

    /// NIDs the HLE dispatcher saw that no module claimed, with per-NID
    /// call counts. Non-empty after a run is a punch list of
    /// unimplemented library entries the scenario touched.
    #[inline]
    pub fn hle_unclaimed_nids(&self) -> &std::collections::BTreeMap<u32, usize> {
        &self.hle.unclaimed_nids
    }

    /// NIDs whose handlers ran but produced no observable mutation
    /// (no set_return, set_register, write_guest, set_unit_finished,
    /// heap_alloc, or alloc_id). A non-empty map is a silent-divergence
    /// punch list: stale register state leaks through to the guest.
    #[inline]
    pub fn hle_handlers_without_mutation(&self) -> &std::collections::BTreeMap<u32, usize> {
        &self.hle.handlers_without_mutation
    }

    /// Set the runtime mode.
    pub fn set_mode(&mut self, mode: RuntimeMode) {
        self.mode = mode;
    }

    /// Current runtime mode.
    pub fn mode(&self) -> RuntimeMode {
        self.mode
    }

    /// Split-borrow the unit and mailbox registries together.
    #[inline]
    pub fn registries_mut(&mut self) -> (&mut UnitRegistry, &mut MailboxRegistry) {
        (&mut self.registry, &mut self.mailbox_registry)
    }

    /// Takes effect on the next `step()` call. Use
    /// [`default_budget_for_mode`] for the natural budget for a mode.
    pub fn set_budget(&mut self, budget: Budget) {
        self.budget_per_step = budget;
    }

    /// Per-step budget grant.
    pub fn budget(&self) -> Budget {
        self.budget_per_step
    }

    /// FNV-1a merge of every sync / committed-state source the runtime
    /// owns (mailboxes, signal registers, LV2 host, syscall responses,
    /// reservations, RSX cursor / semaphore offset / flip state) in a
    /// fixed order. Replay tooling compares pairs via the `SyncState`
    /// checkpoint emitted at every commit boundary.
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

    /// Committed guest memory.
    #[inline]
    pub fn memory(&self) -> &GuestMemory {
        &self.memory
    }

    /// When set, `_cellGcmInitBody` places the control register in the
    /// reserved RSX region so the first put-pointer write trips a
    /// ReservedWrite commit error the CLI translates to a checkpoint.
    pub fn set_gcm_rsx_checkpoint(&mut self, enabled: bool) {
        self.hle.gcm.rsx_checkpoint = enabled;
    }

    /// Mutable committed guest memory.
    #[inline]
    pub fn memory_mut(&mut self) -> &mut GuestMemory {
        &mut self.memory
    }

    /// Atomic-reservation table.
    #[inline]
    pub fn reservations(&self) -> &cellgov_sync::ReservationTable {
        &self.reservations
    }

    /// Mutable atomic-reservation table.
    #[inline]
    pub fn reservations_mut(&mut self) -> &mut cellgov_sync::ReservationTable {
        &mut self.reservations
    }

    /// RSX FIFO cursor.
    #[inline]
    pub fn rsx_cursor(&self) -> &crate::rsx::RsxFifoCursor {
        &self.rsx_cursor
    }

    /// Mutable RSX FIFO cursor.
    #[inline]
    pub fn rsx_cursor_mut(&mut self) -> &mut crate::rsx::RsxFifoCursor {
        &mut self.rsx_cursor
    }

    /// Current RSX semaphore-offset register.
    #[inline]
    pub fn rsx_sem_offset(&self) -> u32 {
        self.rsx_sem_offset
    }

    /// Mutable RSX semaphore-offset register.
    #[inline]
    pub fn rsx_sem_offset_mut(&mut self) -> &mut u32 {
        &mut self.rsx_sem_offset
    }

    /// Enabling this requires the host to have made the RSX region
    /// writable; otherwise every put-pointer store reserved-writes and
    /// the mirror never runs.
    pub fn set_rsx_mirror_writes(&mut self, enabled: bool) {
        self.rsx_mirror_writes = enabled;
    }

    /// Current value of the RSX mirror flag.
    #[inline]
    pub fn rsx_mirror_writes_enabled(&self) -> bool {
        self.rsx_mirror_writes
    }

    /// RSX flip-status state.
    #[inline]
    pub fn rsx_flip(&self) -> &crate::rsx::flip::RsxFlipState {
        &self.rsx_flip
    }

    /// Mutable RSX flip-status state.
    #[inline]
    pub fn rsx_flip_mut(&mut self) -> &mut crate::rsx::flip::RsxFlipState {
        &mut self.rsx_flip
    }

    /// Mirror committed writes to `0xC000_0040..0xC000_004C` into
    /// [`Self::rsx_cursor`]. Reads bytes from committed memory rather
    /// than the effect payload: a partial-overlap write may cross slots
    /// and the authoritative value is what the pipeline applied. Only
    /// full 4-byte slot coverage mirrors -- a sub-word store still
    /// applies to memory but leaves the cursor alone.
    ///
    /// Called from `commit_step` after the batch applies and before the
    /// FIFO advance pass, so the drain sees the new put / ref in the
    /// same batch.
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
                    let Some(slot_range) =
                        cellgov_mem::ByteRange::new(cellgov_mem::GuestAddr::new(slot_start), 4)
                    else {
                        continue;
                    };
                    if let Some(bytes) = self.memory.read(slot_range) {
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

    /// Consume the runtime and return its guest memory. Used to chain
    /// execution phases: run one runtime, extract the initialized memory,
    /// and seed a fresh runtime for the next phase.
    pub fn into_memory(self) -> GuestMemory {
        self.memory
    }

    /// Current guest time.
    #[inline]
    pub fn time(&self) -> GuestTicks {
        self.time
    }

    /// Advances only at commit boundaries; `step()` never advances it.
    #[inline]
    pub fn epoch(&self) -> Epoch {
        self.epoch
    }

    /// Total number of successful `step()` calls so far.
    #[inline]
    pub fn steps_taken(&self) -> usize {
        self.steps_taken
    }

    /// Configured deadlock-detector cap.
    #[inline]
    pub fn max_steps(&self) -> usize {
        self.max_steps
    }

    /// Select a unit, grant budget, run it to yield, advance guest time.
    ///
    /// # Errors
    ///
    /// - [`StepError::MaxStepsExceeded`] -- deadlock detector tripped.
    /// - [`StepError::NoRunnableUnit`] -- terminal stall (nothing can wake).
    /// - [`StepError::AllBlocked`] -- at least one unit parked, none runnable.
    /// - [`StepError::TimeOverflow`] -- consumed budget would push guest
    ///   time past `u64::MAX`.
    ///
    /// Emitted effects are returned verbatim in [`RuntimeStep::result`];
    /// [`Runtime::commit_step`] drives the commit pipeline over them.
    pub fn step(&mut self) -> Result<RuntimeStep, StepError> {
        if self.steps_taken >= self.max_steps {
            return Err(StepError::MaxStepsExceeded);
        }

        let unit_id = match self.scheduler.select_next(&self.registry) {
            Some(id) => id,
            None => {
                // Distinguish terminal stall from soft-stall (parked
                // units that could be woken by a future signal).
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

        // Clear any runtime-side status override so the unit's own status
        // logic resumes after this step.
        self.registry.clear_status_override(unit_id);

        // Per-step trace records only emit in FullTrace.
        if self.mode == RuntimeMode::FullTrace {
            self.trace.record(&TraceRecord::UnitScheduled {
                unit: unit_id,
                granted_budget: self.budget_per_step,
                time: self.time,
                epoch: self.epoch,
            });
        }

        // The memory borrow is alive only for `run_until_yield`,
        // enforcing the freeze-during-step rule. Drain any messages /
        // syscall returns the commit pipeline delivered to this unit.
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
            let ctx = ctx
                .with_reservations(&self.reservations)
                .with_current_tick(self.time);
            let unit = self
                .registry
                .get_mut(unit_id)
                .expect("scheduler returned an id that is not in the registry");
            let res = unit.run_until_yield(self.budget_per_step, &ctx, &mut effects_buf);
            // FaultDriven has no downstream consumer for the drained
            // fingerprints / snapshots, so skip both vtable dispatches.
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

        // PpuStateHash and PpuStateFull pair by step index so the diff
        // printer can match a hash divergence with its full-state
        // snapshot when both are present. Step indices are monotonic
        // and independent of `steps_taken`.
        let hash_base = self.per_step_index;
        for (pc, hash) in retired_hashes {
            self.trace.record(&TraceRecord::PpuStateHash {
                step: self.per_step_index,
                pc,
                hash: cellgov_trace::StateHash::new(hash),
            });
            self.per_step_index += 1;
        }
        // Stamp with `hash_base + i`: a window starting at the unit's
        // first retired instruction aligns `step` fields with the hash
        // stream; mid-run windows carry correct PCs but not step parity
        // -- the diff printer matches by PC in that case.
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

        // Identity time-advancement: budget units map 1:1 to guest ticks.
        let advance = GuestTicks::new(result.consumed_budget.raw());
        let time_after = self
            .time
            .checked_add(advance)
            .ok_or(StepError::TimeOverflow)?;
        self.time = time_after;
        self.steps_taken += 1;
        self.last_scheduled_unit = Some(unit_id);

        if self.mode == RuntimeMode::FullTrace {
            self.trace.record(&TraceRecord::StepCompleted {
                unit: unit_id,
                yield_reason: traced_yield_reason(result.yield_reason),
                consumed_budget: result.consumed_budget,
                time_after,
            });

            for (sequence, effect) in effects_buf.iter().enumerate() {
                self.trace.record(&TraceRecord::EffectEmitted {
                    unit: unit_id,
                    sequence: sequence as u32,
                    kind: traced_effect_kind(effect),
                });
            }
        }

        // Hand `effects_buf` off to `RuntimeStep`; the fresh Vec avoids
        // allocating in the common zero-effects FaultDriven case.
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
