//! Fault diagnostics and the rollback a mid-batch fault takes.

use super::batch::BatchEntry;
use super::ppu_unit::PpuExecutionUnit;
use cellgov_effects::{Effect, FaultKind};
use cellgov_exec::{
    ExecutionStepResult, FaultRegisterDump, LocalDiagnostics, UnitStatus, YieldReason,
};
use cellgov_time::InstructionCost;

impl PpuExecutionUnit {
    fn capture_regs(&self) -> FaultRegisterDump {
        FaultRegisterDump {
            gprs: *self.state.gpr.as_array(),
            lr: self.state.lr(),
            ctr: self.state.ctr(),
            xer: self.state.xer(),
            cr: self.state.cr(),
        }
    }

    pub(super) fn fault_diag(&self, pc: u64) -> LocalDiagnostics {
        LocalDiagnostics {
            pc: Some(pc),
            lr: Some(self.state.lr()),
            syscall_lev: None,
            faulting_ea: None,
            fault_regs: Some(self.capture_regs()),
        }
    }

    pub(super) fn fault_diag_ea(&self, pc: u64, ea: u64) -> LocalDiagnostics {
        LocalDiagnostics {
            pc: Some(pc),
            lr: Some(self.state.lr()),
            syscall_lev: None,
            faulting_ea: Some(ea),
            fault_regs: Some(self.capture_regs()),
        }
    }

    /// Fault-discards-all: restore the batch-entry state, drop staged
    /// stores, fetch runs and effects, rewind the per-step trace to
    /// the batch entry, and mark the unit faulted.
    ///
    /// Retirements discarded by the rollback never reach the trace
    /// stream: their `PpuStateHash` / `PpuStateFull` entries are
    /// truncated and `retirement_counter` is restored.
    fn discard_batch(&mut self, entry: &BatchEntry, effects: &mut Vec<Effect>) {
        if let Some(snap) = entry.snapshot.as_ref() {
            self.state = snap.clone();
        }
        self.store_buf.clear();
        self.drop_fetch_runs();
        effects.clear();
        self.per_step_hashes.truncate(entry.hashes);
        self.per_step_full_states.truncate(entry.fulls);
        self.retirement_counter = entry.retired;
        self.status = UnitStatus::Faulted;
    }

    /// Roll the batch back and yield a guest fault carrying `code`.
    ///
    /// `diag` is a parameter because the rollback erases the fault
    /// site's registers; the caller captures it first.
    pub(super) fn fault_yield(
        &mut self,
        entry: &BatchEntry,
        effects: &mut Vec<Effect>,
        diag: LocalDiagnostics,
        code: u32,
    ) -> ExecutionStepResult {
        self.discard_batch(entry, effects);
        ExecutionStepResult {
            yield_reason: YieldReason::Fault,
            consumed_cost: InstructionCost::ZERO,
            local_diagnostics: diag,
            fault: Some(FaultKind::Guest(code)),
            syscall_args: None,
        }
    }
}
