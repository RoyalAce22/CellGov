//! [`Runtime::commit_step`] -- drives the commit pipeline for a
//! previously-returned step result, then runs the FIFO advance pass
//! and emits the commit trace record.

use cellgov_effects::Effect;
use cellgov_event::UnitId;
use cellgov_exec::{ExecutionStepResult, UnitStatus, YieldReason};

use crate::commit::{BlockReason, CommitContext, CommitError, CommitOutcome};
use crate::runtime::state::Runtime;

impl Runtime {
    /// Drive the commit pipeline for a previously-returned step result.
    ///
    /// Epoch advances on every commit boundary including validation
    /// failures, so an `Err` return still mutates `self.epoch`. Fault
    /// and atomic-batch semantics: see
    /// [`crate::commit::CommitPipeline::process`].
    pub fn commit_step(
        &mut self,
        result: &ExecutionStepResult,
        effects: &[Effect],
    ) -> Result<CommitOutcome, CommitError> {
        self.step_woke_others = false;
        // Trivial-step fast path under FaultDriven. Epoch still advances
        // to preserve the atomic-batch boundary; trace is off in this mode.
        // The RSX half of the predicate must be the exact negation of the
        // slow-path advance trigger (`get != put || !call_stack.is_empty()`),
        // or a mid-CALL state with `get == put` would skip the drain.
        if self.mode == crate::runtime::types::RuntimeMode::FaultDriven
            && effects.is_empty()
            && result.fault.is_none()
            && result.yield_reason.allows_trivial_fast_path()
            && self.dma_queue.is_empty()
            && self.pending_rsx_effects.is_empty()
            && self.rsx_cursor.get() == self.rsx_cursor.put()
            && self.rsx_call_stack.is_empty()
            && !self.rsx_flip.pending()
        {
            self.epoch.advance();
            if let Some(unit) = self.last_scheduled_unit {
                debug_assert!(
                    !self.step_woke_others,
                    "fast path reached with step_woke_others=true; \
                     allows_trivial_fast_path() must exclude every yield reason \
                     that can set the flag (Syscall, sync wakes)",
                );
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

        // `rsx_label_writes_committed` is threaded through CommitContext
        // so `process()` increments it adjacent to the guard it witnesses.
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
            rsx_label_writes_committed: &mut self.rsx_label_writes_committed,
        };
        let mut outcome = self.commit_pipeline.process(result, effects, &mut ctx);

        // Invalidate predecoded caches overlapping committed writes.
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

        // `Runtime::step` sets `last_scheduled_unit` before every
        // commit_step; the unit-0 fallback exists only because release
        // builds must not panic on a `None` here.
        debug_assert!(
            self.last_scheduled_unit.is_some(),
            "commit_step slow path reached with last_scheduled_unit=None; \
             Runtime::step is supposed to set this before every commit_step",
        );
        let source = self.last_scheduled_unit.unwrap_or_else(|| UnitId::new(0));
        if result.yield_reason == YieldReason::Syscall {
            self.dispatch_syscall(result, source);
        }
        // Park before firing completions: fire_dma_completions sets the
        // wake override (Runnable) for any issuer whose completion just
        // landed, which overwrites this Blocked override iff a tag bit
        // got published. Reverse order would leave the SPU Blocked even
        // when its wake just fired.
        if result.yield_reason == YieldReason::DmaWait {
            self.registry
                .set_status_override(source, UnitStatus::Blocked);
            if let Ok(ref mut o) = outcome {
                o.blocked_units.push((source, BlockReason::DmaWait));
            }
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
        //
        // Bring-up GET catch-up: reconcile `cursor.get` with the title's
        // MMIO GET before invoking the walker, so the walker starts from
        // the read position libgcm staged at FIFO bring-up. Ownership
        // rationale on [`Self::mirror_rsx_control_register_writes`].
        if self.rsx_consume_fifo {
            self.catch_up_cursor_get_from_mmio();
        }
        if self.rsx_cursor.get() != self.rsx_cursor.put() || !self.rsx_call_stack.is_empty() {
            let rsx_ctx = self.lv2_host.sys_rsx_context();
            let iomap = crate::rsx::IoMap {
                ea: rsx_ctx.iomap_ea,
                io: rsx_ctx.iomap_io,
                size: rsx_ctx.iomap_size,
            };
            let advance_outcome = crate::rsx::advance::rsx_advance(
                &self.memory,
                &iomap,
                &mut self.rsx_cursor,
                &mut self.rsx_sem_offset,
                &mut self.rsx_call_stack,
                &self.rsx_methods,
                &mut self.pending_rsx_effects,
                self.time,
            );
            self.rsx_set_reference_dispatches = self
                .rsx_set_reference_dispatches
                .wrapping_add(u64::from(advance_outcome.set_references_dispatched));

            // Gated on rsx_consume_fifo so reserved-region titles keep
            // their MMIO slots untouched when the consumer is off.
            if self.rsx_consume_fifo && advance_outcome.reached_put() {
                self.mirror_rsx_cursor_to_mmio();
                self.assert_ref_addr_mirrors_cursor();
            }
        }

        if flip_pending_at_entry {
            self.rsx_flip.complete_pending_flip();
        }

        // Flip-status memory mirror. `rsx_flip` is the authoritative
        // model; the mirror at `RSX_FLIP_STATUS_MIRROR_ADDR` is a
        // best-effort projection for titles that poll the address.
        // On failure the projection is dropped (typed invariant break)
        // but the model advance stands -- rolling `rsx_flip` back to
        // match a failed projection would corrupt the source of truth.
        if self.rsx_mirror_writes {
            let flip_status_now = self.rsx_flip.status();
            if flip_status_now != flip_status_at_entry {
                let addr = crate::rsx::RSX_FLIP_STATUS_MIRROR_ADDR as u64;
                let range = cellgov_mem::ByteRange::new(cellgov_mem::GuestAddr::new(addr), 4)
                    .expect(
                        "RSX_FLIP_STATUS_MIRROR_ADDR is a 4-byte aligned constant; \
                         ByteRange::new on a fixed 4-byte slot cannot misalign or overflow",
                    );
                let value = flip_status_now as u32;
                if let Err(err) = self.memory.apply_commit(range, &value.to_be_bytes()) {
                    self.lv2_host.log_invariant_break(
                        "dispatch.rsx_flip_status_mirror_failed",
                        format_args!(
                            "RSX flip-status mirror write failed at \
                             addr=0x{addr:016x} length=4: {err}; \
                             rsx_flip model advance retained, guest-visible \
                             mirror byte stale",
                        ),
                    );
                }
            }
        }

        self.emit_commit_trace(source, &outcome, &due);

        let holds_cs = self.lv2_host.unit_holds_lwmutex(source);
        self.scheduler
            .notify_yielded(source, result.yield_reason, self.step_woke_others, holds_cs);

        outcome
    }

    /// Monotonic one-shot GET catch-up: advance `cursor.get` to
    /// `mem[GET_ADDR]` iff the MMIO value is strictly greater.
    /// Ownership rationale on
    /// [`Self::mirror_rsx_control_register_writes`].
    fn catch_up_cursor_get_from_mmio(&mut self) {
        use crate::rsx::control_register::GET_ADDR;
        let range = cellgov_mem::ByteRange::new(cellgov_mem::GuestAddr::new(GET_ADDR as u64), 4)
            .expect(
                "control_register::GET_ADDR is a 4-byte aligned constant; \
                 ByteRange::new on a fixed 4-byte slot cannot misalign or overflow",
            );
        let Some(bytes) = self.memory.read(range) else {
            return;
        };
        let mmio_get = u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
        if mmio_get > self.rsx_cursor.get() {
            self.rsx_cursor.set_get(mmio_get);
        }
    }

    /// Post-writeback invariant: `mem[REF_ADDR]` equals
    /// `cursor.current_reference()`. Debug builds panic; release
    /// builds emit a typed invariant break.
    fn assert_ref_addr_mirrors_cursor(&mut self) {
        use crate::rsx::control_register::REF_ADDR;
        let expected = self.rsx_cursor.current_reference();
        let range = cellgov_mem::ByteRange::new(cellgov_mem::GuestAddr::new(REF_ADDR as u64), 4)
            .expect(
                "control_register::REF_ADDR is a 4-byte aligned constant; \
                 ByteRange::new on a fixed 4-byte slot cannot misalign or overflow",
            );
        let Some(bytes) = self.memory.read(range) else {
            self.lv2_host.log_invariant_break(
                "dispatch.rsx_ref_addr_post_writeback_unmapped",
                format_args!(
                    "post-writeback read of REF_ADDR returned None (range unmapped); \
                     expected cursor.current_reference() = 0x{expected:08x}",
                ),
            );
            return;
        };
        let observed = u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
        debug_assert!(
            observed == expected,
            "mem[REF_ADDR] = 0x{observed:08x} but cursor.current_reference() = 0x{expected:08x} \
             after mirror_rsx_cursor_to_mmio; the writeback dropped or was clobbered",
        );
        if observed != expected {
            self.lv2_host.log_invariant_break(
                "dispatch.rsx_ref_addr_post_writeback_mismatch",
                format_args!(
                    "mem[REF_ADDR] = 0x{observed:08x} after writeback but cursor.current_reference() \
                     = 0x{expected:08x}; the writeback was dropped or clobbered",
                ),
            );
        }
    }

    /// Project the cursor's `current_reference` and `get` into the
    /// MMIO control-register slots (`REF_ADDR`, `GET_ADDR`). The
    /// title's libgcm spin-poll on `dma.ref` clears when the
    /// SET_REFERENCE value it baked into the FIFO stream lands here.
    /// On a failed write the projection is dropped (typed invariant
    /// break) and the cursor model advance stands.
    fn mirror_rsx_cursor_to_mmio(&mut self) {
        use crate::rsx::control_register;
        let writes = [
            (
                control_register::REF_ADDR,
                self.rsx_cursor.current_reference(),
            ),
            (control_register::GET_ADDR, self.rsx_cursor.get()),
        ];
        for (addr, value) in writes {
            let range = cellgov_mem::ByteRange::new(cellgov_mem::GuestAddr::new(addr as u64), 4)
                .expect(
                    "control_register::{REF,GET}_ADDR are 4-byte aligned constants; \
                     ByteRange::new on a fixed 4-byte slot cannot misalign or overflow",
                );
            if let Err(err) = self.memory.apply_commit(range, &value.to_be_bytes()) {
                self.lv2_host.log_invariant_break(
                    "dispatch.rsx_cursor_mmio_writeback_failed",
                    format_args!(
                        "RSX cursor->MMIO writeback failed at addr=0x{addr:08x} value=0x{value:08x}: {err}; \
                         cursor model advance retained, guest-visible MMIO byte stale",
                    ),
                );
            }
        }
    }

    /// Mirror committed guest writes to `put` and `ref` (`0xC000_0040`,
    /// `0xC000_0048`) into [`Self::rsx_cursor`]. Reads from committed
    /// memory rather than the effect payload so partial-overlap writes
    /// resolve to the value the pipeline applied. Only full 4-byte slot
    /// coverage mirrors; sub-word stores still apply to memory but
    /// leave the cursor alone.
    ///
    /// `get` (`0xC000_0044`) is NOT mirrored here. The walker owns
    /// `get` in steady state (RPCS3 `Emu/RSX/NV47/HW/nv406e.cpp:19`
    /// writes `dma.get` from the engine at every SET_REFERENCE
    /// dispatch); the CPU writes it once at FIFO bring-up to seed the
    /// initial read position. [`Self::catch_up_cursor_get_from_mmio`]
    /// picks up that seed at walker invocation, monotonically -- a
    /// per-effect mirror here would let a mid-walk guest GET write
    /// yank the cursor backward against an active walker. The reverse
    /// projection lives in [`Self::mirror_rsx_cursor_to_mmio`].
    ///
    /// Runs after the batch applies and before the FIFO advance pass, so
    /// the drain sees the new put / ref in the same batch.
    fn mirror_rsx_control_register_writes(&mut self, effects: &[Effect]) {
        use crate::rsx::control_register;
        enum Slot {
            Put,
            Ref,
        }
        const SLOTS: [(u32, Slot); 2] = [
            (control_register::PUT_ADDR, Slot::Put),
            (control_register::REF_ADDR, Slot::Ref),
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
                    let slot_range =
                        cellgov_mem::ByteRange::new(cellgov_mem::GuestAddr::new(slot_start), 4)
                            .expect(
                                "control_register::{PUT,REF}_ADDR are 4-byte aligned constants; \
                         ByteRange::new on a fixed 4-byte slot cannot misalign or overflow",
                            );
                    if let Some(bytes) = self.memory.read(slot_range) {
                        let value = u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
                        match slot {
                            Slot::Put => self.rsx_cursor.set_put(value),
                            Slot::Ref => self.rsx_cursor.set_reference(value),
                        }
                    }
                }
            }
        }
    }

    #[cfg(all(test, debug_assertions))]
    pub(crate) fn test_only_assert_ref_addr_mirrors_cursor(&mut self) {
        self.assert_ref_addr_mirrors_cursor();
    }

    #[cfg(test)]
    pub(crate) fn test_only_catch_up_cursor_get_from_mmio(&mut self) {
        self.catch_up_cursor_get_from_mmio();
    }
}

#[cfg(test)]
#[path = "tests/commit_step_tests.rs"]
mod tests;
