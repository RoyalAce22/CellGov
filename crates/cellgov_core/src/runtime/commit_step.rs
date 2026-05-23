//! [`Runtime::commit_step`] -- drives the commit pipeline for a
//! previously-returned step result, then runs the FIFO advance pass
//! and emits the commit trace record.

use cellgov_effects::Effect;
use cellgov_event::UnitId;
use cellgov_exec::{ExecutionStepResult, YieldReason};

use crate::commit::{CommitContext, CommitError, CommitOutcome};
use crate::runtime::state::Runtime;

impl Runtime {
    /// Drive the commit pipeline for a previously-returned step result.
    ///
    /// Epoch advances on every commit boundary including validation
    /// failures: the step's effect set is closed either way, so an `Err`
    /// return still mutates `self.epoch`. Fault rule and atomic-batch
    /// semantics are inherited from [`crate::commit::CommitPipeline::process`].
    pub fn commit_step(
        &mut self,
        result: &ExecutionStepResult,
        effects: &[Effect],
    ) -> Result<CommitOutcome, CommitError> {
        self.step_woke_others = false;
        // Trivial-step fast path under FaultDriven. Epoch still advances
        // to preserve the atomic-batch boundary; trace is off in this mode.
        if self.mode == crate::runtime::types::RuntimeMode::FaultDriven
            && effects.is_empty()
            && result.fault.is_none()
            && result.yield_reason.allows_trivial_fast_path()
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
}
