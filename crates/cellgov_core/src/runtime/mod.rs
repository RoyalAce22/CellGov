//! Step driver and commit pipeline over registered units and guest memory.
//!
//! Determinism contract: every observable output is a pure function of
//! runtime state + registry contents + scheduler decisions. No host time,
//! no `HashMap` iteration. The `max_steps` cap trips
//! [`StepError::MaxStepsExceeded`] rather than looping on a stalled system.
//!
//! The main trace stream is fixed-size-per-record; full PPU register
//! snapshots route to `zoom_trace` to keep the main stream homogeneous.

use crate::commit::{CommitContext, CommitError, CommitOutcome, CommitPipeline};
use crate::registry::UnitRegistry;
use crate::scheduler::Scheduler;
use crate::syscall_table::SyscallResponseTable;
use cellgov_dma::{DmaLatencyModel, DmaQueue};
use cellgov_effects::Effect;
use cellgov_event::UnitId;
use cellgov_exec::{ExecutionContext, ExecutionStepResult, YieldReason};
use cellgov_lv2::Lv2Host;
use cellgov_mem::GuestMemory;
use cellgov_sync::{MailboxRegistry, SignalRegistry};
use cellgov_time::{Budget, Epoch, GuestTicks};
use cellgov_trace::{TraceRecord, TraceWriter};

mod accessors;
mod commit_trace;
mod construction;
mod dma;
mod lv2_dispatch;
mod mem_helpers;
mod ppu_create;
mod snapshot;
mod sync_wakes;
mod trace_bridge;
mod types;
pub use snapshot::RuntimeSnapshot;
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
    rsx_cursor: crate::rsx::RsxFifoCursor,
    /// Persists across commit boundaries: an OFFSET / RELEASE pair may
    /// straddle drains and the later RELEASE must read the earlier OFFSET.
    rsx_sem_offset: u32,
    /// Host must make the RSX region writable before enabling; otherwise
    /// every put-pointer store reserved-writes and the mirror never runs.
    rsx_mirror_writes: bool,
    rsx_flip: crate::rsx::flip::RsxFlipState,
    rsx_methods: crate::rsx::method::NvMethodTable,
    /// Advance-pass effects produced at the end of commit batch N, queued
    /// for the start of batch N+1. FIFO method parses mutate cursor +
    /// sem_offset in batch N; downstream memory / state effects commit
    /// alongside batch N+1 to preserve the atomic-batch contract.
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
    /// One commit batch per unit yield; attributes the batch to its source.
    last_scheduled_unit: Option<UnitId>,
    /// True when the just-completed step's dispatch transitioned at least
    /// one other unit into `Runnable`. Surfaced via `notify_yielded` so
    /// scheduler policy can distinguish wake-causing syscalls (`sema_post`,
    /// `event_flag_set`) from non-waking ones (`tty_write`,
    /// `ppu_thread_get_id`).
    step_woke_others: bool,
    /// Base address used by `RsxLabelWrite` effects when computing the
    /// commit-side guest address (`base + offset`). Zero means RSX has
    /// not allocated label memory yet; synthetic scenarios set it
    /// directly via [`Runtime::set_rsx_label_base`].
    rsx_label_base: u32,
    effects_buf: Vec<Effect>,
    mode: RuntimeMode,
    /// Monotonic over per-instruction state hashes; orthogonal to
    /// `steps_taken`, which counts `run_until_yield` invocations.
    per_step_index: u64,
    zoom_trace: TraceWriter,
    /// Set by [`Runtime::restore_into`], cleared by
    /// [`Runtime::set_scheduler`]; [`Runtime::step`] debug-panics
    /// if it sees this set. Catches stepping with a scheduler whose
    /// internal sticky-streak / last-position state was carried over
    /// from before the restore.
    scheduler_dirty_after_restore: bool,
}

impl Runtime {
    /// Drive the commit pipeline for a previously-returned step result.
    ///
    /// Epoch advances on every commit boundary including validation
    /// failures: the step's effect set is closed either way, so an `Err`
    /// return still mutates `self.epoch`. Fault rule and atomic-batch
    /// semantics are inherited from [`CommitPipeline::process`].
    pub fn commit_step(
        &mut self,
        result: &ExecutionStepResult,
        effects: &[Effect],
    ) -> Result<CommitOutcome, CommitError> {
        self.step_woke_others = false;
        // Trivial-step fast path under FaultDriven. Epoch still advances
        // to preserve the atomic-batch boundary; trace is off in this mode.
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
            self.epoch.advance();
            if let Some(unit) = self.last_scheduled_unit {
                let holds_cs = self.lv2_host.unit_holds_lwmutex(unit);
                self.scheduler
                    .notify_yielded(unit, result.yield_reason, false, holds_cs);
            }
            return Ok(CommitOutcome::default());
        }

        // Prepend RSX effects from the previous commit's advance pass.
        // Allocates only when pending_rsx_effects is non-empty.
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

        // Snapshot so the post-apply DONE transition fires only for flips
        // pending at entry; a flip queued in this batch must be observable
        // as WAITING for at least one PPU step before completing.
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
            rsx_label_base: self.rsx_label_base,
            rsx_flip: &mut self.rsx_flip,
        };
        let mut outcome = self.commit_pipeline.process(result, effects, &mut ctx);

        // Invalidate predecoded caches overlapping committed writes;
        // required for self-modifying code and runtime relocations.
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
        self.epoch.advance();
        let due = self.fire_dma_completions();
        if let Ok(ref mut o) = outcome {
            o.dma_completions_fired = due.len();
        }

        if result.yield_reason == YieldReason::Finished {
            self.resolve_join_wakes(source);
        }

        // RSX FIFO advance: after unit effects commit and DMA completions
        // fire, before state-hash checkpoints emit. Emitted effects land
        // in `pending_rsx_effects` and commit with the next batch
        // (atomic-batch contract); cursor mutations land in THIS batch's
        // state-hash checkpoint.
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

        if flip_pending_at_entry {
            self.rsx_flip.complete_pending_flip();
        }

        // Flip-status memory mirror; gated on rsx_mirror_writes because
        // the default reserved RSX layout would reserved-fault. No-change
        // skip keeps the memory hash stable across idle batches.
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

        let holds_cs = self.lv2_host.unit_holds_lwmutex(source);
        self.scheduler
            .notify_yielded(source, result.yield_reason, self.step_woke_others, holds_cs);

        outcome
    }

    /// FNV-1a merge over every sync / committed-state source the runtime
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

    /// Mirror committed writes to `0xC000_0040..0xC000_004C` into
    /// [`Self::rsx_cursor`]. Reads from committed memory rather than the
    /// effect payload: partial-overlap writes may cross slots and the
    /// authoritative value is what the pipeline applied. Only full 4-byte
    /// slot coverage mirrors; sub-word stores still apply to memory but
    /// leave the cursor alone.
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
        debug_assert!(
            !self.scheduler_dirty_after_restore,
            "Runtime::step called between restore_into and set_scheduler; the snapshotted \
             last_scheduled_unit / step_woke_others would diverge from the scheduler's stale \
             internal sticky-streak counter. Install a fresh scheduler after every restore_into."
        );
        if self.steps_taken >= self.max_steps {
            return Err(StepError::MaxStepsExceeded);
        }

        let unit_id = match self.scheduler.select_next(&self.registry) {
            Some(id) => id,
            None => {
                // Distinguish terminal stall from soft-stall: parked units
                // could still wake from a future signal.
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

        // Clear runtime-side status override so the unit's own status
        // logic resumes for this step.
        self.registry.clear_status_override(unit_id);

        if self.mode == RuntimeMode::FullTrace {
            self.trace.record(&TraceRecord::UnitScheduled {
                unit: unit_id,
                granted_budget: self.budget_per_step,
                time: self.time,
                epoch: self.epoch,
            });
        }

        // Memory borrow scoped to `run_until_yield` to enforce the
        // freeze-during-step rule. Drains messages / syscall returns the
        // commit pipeline delivered to this unit.
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
                .with_current_tick(self.time)
                .with_trace_per_step(self.mode != RuntimeMode::FaultDriven);
            let unit = self
                .registry
                .get_mut(unit_id)
                .expect("scheduler returned an id that is not in the registry");
            let res = unit.run_until_yield(self.budget_per_step, &ctx, &mut effects_buf);
            // FaultDriven has no consumer for fingerprints / snapshots;
            // skip both vtable dispatches.
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
        // printer matches a hash divergence with its full-state snapshot.
        // Step indices are monotonic and independent of `steps_taken`.
        let hash_base = self.per_step_index;
        for (pc, hash) in retired_hashes {
            self.trace.record(&TraceRecord::PpuStateHash {
                step: self.per_step_index,
                pc,
                hash: cellgov_trace::StateHash::new(hash),
            });
            self.per_step_index += 1;
        }
        // `hash_base + i` aligns `step` with the hash stream when the
        // window starts at the unit's first retired instruction. Mid-run
        // windows carry correct PCs but not step parity -- the diff
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

        let advance: GuestTicks = result.consumed_cost.into();
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
                yield_reason: traced_yield_reason(
                    result.yield_reason,
                    result.local_diagnostics.syscall_lev,
                ),
                consumed_cost: result.consumed_cost,
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

        // Hand `effects_buf` off to `RuntimeStep`; the fresh empty Vec
        // avoids allocating in the common zero-effects FaultDriven case.
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
#[path = "../tests/runtime_tests.rs"]
mod tests;
