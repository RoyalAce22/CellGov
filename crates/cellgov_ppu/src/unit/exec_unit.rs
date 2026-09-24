//! The [`ExecutionUnit`] contract the runtime drives the PPU through.

use super::batch::NoTap;
use super::ppu_unit::{PpuExecutionUnit, PpuSnapshot};
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
        PpuSnapshot {
            gpr: *self.state.gpr.as_array(),
            fpr: *self.state.fpr.as_array(),
            vr: *self.state.vr.as_array(),
            pc: self.state.pc,
            cr: self.state.cr(),
            lr: self.state.lr(),
            ctr: self.state.ctr(),
            xer: self.state.xer(),
            tb: self.state.tb,
            reservation_line: self.state.reservation().map(|l| l.addr()),
        }
    }

    fn drain_retired_state_hashes(&mut self) -> Vec<(u64, u64)> {
        std::mem::take(&mut self.per_step_hashes)
    }

    fn drain_retired_state_full(&mut self) -> Vec<(u64, u64, cellgov_exec::PpuFingerprint)> {
        std::mem::take(&mut self.per_step_full_states)
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
