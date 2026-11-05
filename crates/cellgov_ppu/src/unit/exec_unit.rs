//! The [`ExecutionUnit`] contract the runtime drives the PPU through.

use super::batch::NoTap;
use super::ppu_unit::{PpuExecutionUnit, PpuSnapshot};
use crate::observation::PpuArchitecturalState;
use cellgov_effects::Effect;
use cellgov_event::UnitId;
use cellgov_exec::{ExecutionContext, ExecutionStepResult, ExecutionUnit, UnitStatus};
use cellgov_time::Budget;

impl ExecutionUnit for PpuExecutionUnit {
    type Snapshot = PpuSnapshot;

    fn unit_id(&self) -> UnitId {
        self.id
    }

    fn status(&self) -> UnitStatus {
        self.status
    }

    fn run_until_yield(
        &mut self,
        budget: Budget,
        ctx: &ExecutionContext<'_>,
        effects: &mut Vec<Effect>,
    ) -> ExecutionStepResult {
        match self.tap.take() {
            None => self.run_batch(budget, ctx, effects, &NoTap),
            Some(tap) => {
                let result = self.run_batch(budget, ctx, effects, tap.as_ref());
                self.tap = Some(tap);
                result
            }
        }
    }

    fn snapshot(&self) -> PpuSnapshot {
        // Every field is named, so a new one fails to compile here until
        // the snapshot takes it or says why not.
        let PpuExecutionUnit {
            // The unit's identity, which a restore keeps.
            id: _,
            state,
            status,
            break_pc,
            break_skip,
            // Trace output the runtime drains.
            per_step_hashes: _,
            per_step_full_states: _,
            barriers: _,
            code_events: _,
            // Trace instruments: the step the records carry and the
            // window of full records.
            retirement_counter: _,
            full_state_window: _,
            // A cache of guest memory that `invalidate_code` keeps current.
            instruction_shadow: _,
            shadow_hits: _,
            shadow_misses: _,
            // Batch-local: each batch entry clears them.
            fetch_start: _,
            fetch_end: _,
            fetch_runs: _,
            store_buf: _,
            timing,
            // Profile instruments and the observer.
            profile_mode: _,
            profile_insns: _,
            profile_pairs: _,
            profile_prev: _,
            operand_profile: _,
            load_profile: _,
            tap: _,
        } = self;
        PpuSnapshot {
            state: PpuArchitecturalState::capture(state),
            status: *status,
            break_pc: *break_pc,
            break_skip: *break_skip,
            timing: timing.clone(),
        }
    }

    fn drain_retired_state_hashes(&mut self) -> Vec<(u64, u64)> {
        std::mem::take(&mut self.per_step_hashes)
    }

    fn drain_retired_state_full(&mut self) -> Vec<(u64, u64, cellgov_exec::PpuFingerprint)> {
        std::mem::take(&mut self.per_step_full_states)
    }

    fn drain_barriers(&mut self) -> Vec<cellgov_exec::RetiredBarrier> {
        std::mem::take(&mut self.barriers)
    }

    fn drain_code_events(&mut self) -> Vec<cellgov_exec::CodeEvent> {
        std::mem::take(&mut self.code_events)
    }

    fn drain_profile_insns(&mut self) -> Vec<(&'static str, u64)> {
        let map = std::mem::take(&mut self.profile_insns);
        let mut v: Vec<_> = map.into_iter().collect();
        v.sort_by_key(|e| std::cmp::Reverse(e.1));
        v
    }

    fn drain_profile_pairs(&mut self) -> Vec<((&'static str, &'static str), u64)> {
        let map = std::mem::take(&mut self.profile_pairs);
        let mut v: Vec<_> = map.into_iter().collect();
        v.sort_by_key(|e| std::cmp::Reverse(e.1));
        v
    }

    fn invalidate_code(&mut self, addr: u64, len: u64) {
        if let Some(s) = self.instruction_shadow.as_mut() {
            s.invalidate_range(addr, len);
        }
    }

    fn caches_code(&self) -> bool {
        self.instruction_shadow.is_some()
    }

    fn shares_ppe_issue(&self) -> bool {
        true
    }

    fn time_base_read(&self) -> Option<u64> {
        self.state.clock_read.then_some(self.state.tb)
    }

    fn ppe_issue_pressure(&self) -> cellgov_exec::PpeIssuePressure {
        self.timing.pressure()
    }

    fn shadow_stats(&self) -> (u64, u64) {
        (self.shadow_hits, self.shadow_misses)
    }

    fn register_dump(&self) -> Option<cellgov_exec::FaultRegisterDump> {
        Some(cellgov_exec::FaultRegisterDump {
            gprs: *self.state.gpr.as_array(),
            lr: self.state.lr(),
            ctr: self.state.ctr(),
            xer: self.state.xer(),
            cr: self.state.cr(),
        })
    }
}
