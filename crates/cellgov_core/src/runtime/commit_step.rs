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
        //
        // The RSX-work-pending predicate must exactly mirror the
        // slow-path advance trigger's negation. Slow-path runs the
        // walker when `get != put || !call_stack.is_empty()`; the
        // fast path may skip the walker only when both halves are
        // false. Omitting the call-stack check let a state with
        // `get == put` but a non-empty stack (mid-CALL, awaiting
        // RET) take the fast path and silently skip the drain.
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
                // The hardcoded `false` for step_woke_others is
                // load-bearing: only Syscall / event-flag yields can
                // set the flag, and `allows_trivial_fast_path()`
                // excludes those yield reasons. The assert pins
                // that coupling so a future yield-reason that wakes
                // others without going through Syscall would trip
                // here rather than silently dropping a wake notice.
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

        // Audit C-2 witness is incremented inside `process()` adjacent
        // to the semaphore-region `debug_assert!` it witnesses; pass
        // the runtime field through CommitContext so the count cannot
        // diverge from the guard's evaluation frequency.
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

        // `Runtime::step` always sets `last_scheduled_unit` to the
        // unit it dispatched before calling `commit_step`. The
        // fallback to `UnitId::new(0)` is structurally unreachable
        // on the slow path; pin it so a future control-flow change
        // that elides the assignment trips here rather than
        // silently fabricating unit 0 as the batch source (which
        // is a real valid id and would attribute the commit to
        // it).
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
        // Bring-up GET catch-up: when the consumer is on, reconcile
        // `cursor.get` with the title's MMIO `GET_ADDR` before
        // invoking the walker, monotonically (catch up only; never
        // roll back). libgcm's FIFO bring-up writes the title's
        // declared read position into MMIO GET; without this
        // reconciliation the walker would start from a stale cursor
        // (zero or wherever an earlier advance left it) and bail on
        // the first malformed header in front of where the title
        // actually staged its commands. The "monotonic" qualifier is
        // load-bearing per the rationale on
        // [`Self::mirror_rsx_control_register_writes`]: an
        // unconditional mirror would let a guest GET write yank the
        // cursor backward against an active walker. Once the walker
        // takes ownership of GET (the engine-side `.release(...)`
        // model from RPCS3 `Emu/RSX/NV47/HW/nv406e.cpp:19`), the
        // CPU side does not write a smaller value; if it does, we
        // ignore it.
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

            // 40F cursor->MMIO writeback: when the consumer reaches
            // the FIFO tail cleanly, project (current_reference,
            // get) into the MMIO control-register slots at
            // 0xC000_0048 (dma.ref) and 0xC000_0044 (dma.get). The
            // title's libgcm spin-poll on dma.ref clears when its
            // expected reference value lands; the get writeback
            // keeps the engine-side cursor projection consistent
            // with our walker for any subsequent title-side reads.
            // Gated on rsx_consume_fifo so reserved-region titles
            // don't perturb the MMIO slots when the consumer is
            // off. Mirror-failure handling matches the flip-status
            // pattern: log + retain the model advance.
            if self.rsx_consume_fifo && advance_outcome.reached_put() {
                self.mirror_rsx_cursor_to_mmio();
                // C-6 invariant: after the writeback,
                // `mem[REF_ADDR]` must equal
                // `cursor.current_reference()`. The writeback is
                // a one-line `apply_commit` on a 4-byte aligned
                // slot in a region the manifest marked
                // ReadWrite, so the only failure mode is a
                // pipeline bug (filter, region-access mismatch,
                // batch ordering). Compiler can't check it; the
                // invariant lives here so a regression that
                // silently drops the writeback is loud, not
                // silent. Paired with
                // `mirror_rsx_cursor_to_mmio_invariant_panics_under_debug`
                // (debug-only `#[should_panic]` confirming the
                // assertion fires on a divergent mirror).
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

    /// 40F cursor->MMIO writeback. Projects the cursor's
    /// `current_reference` and `get` slots into MMIO at
    /// `0xC000_0048` and `0xC000_0044`. The title's libgcm spin-poll
    /// on `dma.ref` clears when the SET_REFERENCE value it baked
    /// into the FIFO command stream reaches it via this writeback.
    /// The `get` writeback keeps the engine-side cursor consistent
    /// for any subsequent title-side reads of `dma.get`. Failure to
    /// project follows the flip-status mirror pattern: log + retain
    /// the model advance.
    /// Monotonic one-shot GET catch-up. Reads `mem[GET_ADDR]` and
    /// advances `cursor.get` to it iff the MMIO value is strictly
    /// greater than the current cursor; otherwise leaves the cursor
    /// alone. Called at every walker invocation; the monotonic guard
    /// is the load-bearing piece. See the rewritten design comment on
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
    /// `cursor.current_reference()`. Debug builds panic via
    /// `debug_assert!`; release builds emit a typed invariant break.
    /// Paired test:
    /// `mirror_rsx_cursor_to_mmio_invariant_panics_under_debug`.
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
    /// `get` (`0xC000_0044`) is deliberately NOT mirrored from guest
    /// writes by this general per-effect mirror. Two facts shape this:
    ///
    /// 1. The walker owns `get` in steady state. RPCS3
    ///    `Emu/RSX/NV47/HW/nv406e.cpp:19` writes `dma.get` from the
    ///    engine via `.release(get_pos())` at every SET_REFERENCE
    ///    dispatch; no CPU-side write occurs once the walker is the
    ///    active producer of the cursor.
    /// 2. The CPU seeds the initial read position once at FIFO
    ///    bring-up via the title's MMIO GET write (libgcm's
    ///    pre-bringup phase, before the walker has consumed any
    ///    methods). Without picking up that seed, the walker starts
    ///    at a stale cursor and bails on a malformed header in front
    ///    of where the title actually staged its commands.
    ///
    /// The catch-up at [`Self::catch_up_cursor_get_from_mmio`]
    /// reconciles only at walker invocation, and only monotonically
    /// (never roll the cursor backward). That placement is
    /// load-bearing: an unconditional per-effect mirror here would
    /// let a mid-walk guest GET write yank the cursor backward
    /// against an active walker, the divergence the engine-owned-GET
    /// model is meant to prevent. The engine-to-MMIO `get`
    /// projection (the matching write back into MMIO) lives in
    /// [`Self::mirror_rsx_cursor_to_mmio`].
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
mod tests {
    use cellgov_mem::{GuestAddr, GuestMemory, PageSize, Region};
    use cellgov_time::Budget;

    use crate::Runtime;

    fn make_rt_with_rsx_region() -> Runtime {
        let regions = vec![
            Region::new(0, 0x10000, "flat", PageSize::Page4K),
            Region::new(0xC000_0000, 0x1000, "rsx", PageSize::Page64K),
        ];
        let mem = GuestMemory::from_regions(regions).expect("non-overlapping");
        Runtime::new(mem, Budget::new(1), 100)
    }

    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "after mirror_rsx_cursor_to_mmio")]
    fn assert_ref_addr_mirrors_cursor_panics_when_writeback_was_dropped() {
        let mut rt = make_rt_with_rsx_region();
        rt.rsx_cursor_mut().set_reference(0xFFFF_FFFF);
        rt.test_only_assert_ref_addr_mirrors_cursor();
    }

    #[test]
    fn catch_up_advances_cursor_get_when_mmio_get_is_ahead() {
        use crate::rsx::control_register::GET_ADDR;
        let mut rt = make_rt_with_rsx_region();
        let range = cellgov_mem::ByteRange::new(GuestAddr::new(GET_ADDR as u64), 4).unwrap();
        rt.memory_mut()
            .apply_commit(range, &0x0000_1000u32.to_be_bytes())
            .unwrap();
        rt.rsx_cursor_mut().set_get(0x0000_098c);
        rt.test_only_catch_up_cursor_get_from_mmio();
        assert_eq!(rt.rsx_cursor().get(), 0x0000_1000);
    }

    #[test]
    fn catch_up_leaves_cursor_alone_when_mmio_get_is_behind() {
        use crate::rsx::control_register::GET_ADDR;
        let mut rt = make_rt_with_rsx_region();
        let range = cellgov_mem::ByteRange::new(GuestAddr::new(GET_ADDR as u64), 4).unwrap();
        rt.memory_mut()
            .apply_commit(range, &0x0000_0500u32.to_be_bytes())
            .unwrap();
        rt.rsx_cursor_mut().set_get(0x0000_1000);
        rt.test_only_catch_up_cursor_get_from_mmio();
        assert_eq!(rt.rsx_cursor().get(), 0x0000_1000);
    }

    #[test]
    fn catch_up_leaves_cursor_alone_when_mmio_get_equals_cursor() {
        use crate::rsx::control_register::GET_ADDR;
        let mut rt = make_rt_with_rsx_region();
        let range = cellgov_mem::ByteRange::new(GuestAddr::new(GET_ADDR as u64), 4).unwrap();
        rt.memory_mut()
            .apply_commit(range, &0x0000_1000u32.to_be_bytes())
            .unwrap();
        rt.rsx_cursor_mut().set_get(0x0000_1000);
        rt.test_only_catch_up_cursor_get_from_mmio();
        assert_eq!(rt.rsx_cursor().get(), 0x0000_1000);
    }
}
