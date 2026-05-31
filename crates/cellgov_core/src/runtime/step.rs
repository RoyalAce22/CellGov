//! [`Runtime::step`] -- select a unit, grant budget, run it to yield,
//! advance guest time, emit trace records.

use cellgov_exec::ExecutionContext;
use cellgov_time::GuestTicks;
use cellgov_trace::TraceRecord;

use crate::runtime::state::Runtime;
use crate::runtime::trace_bridge::traced_effect_kind;
use crate::runtime::trace_bridge::traced_yield_reason;
use crate::runtime::types::{RuntimeMode, RuntimeStep, StepError};

impl Runtime {
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

        if let Some((addr, width)) = cellgov_mem::value_sample::pending(self.steps_taken as u64) {
            let bytes =
                cellgov_mem::ByteRange::new(cellgov_mem::GuestAddr::new(addr), u64::from(width))
                    .and_then(|range| self.memory.read(range));
            cellgov_mem::value_sample::emit(self.steps_taken as u64, bytes);
        }

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
