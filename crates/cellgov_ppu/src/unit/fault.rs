//! Fault diagnostics and the rollback of a batch that faults with
//! nothing retired.

use super::batch::BatchEntry;
use super::ppu_unit::PpuExecutionUnit;
use crate::exec::PpuFault;
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

    /// The diagnostics of `fault` at `pc`, with the registers at that
    /// point and, for a PPE interrupt, the interrupt registers.
    pub(super) fn fault_diag(&self, pc: u64, fault: &PpuFault) -> LocalDiagnostics {
        let mut diag = LocalDiagnostics {
            pc: Some(pc),
            lr: Some(self.state.lr()),
            syscall_lev: None,
            faulting_ea: None,
            fault_regs: Some(self.capture_regs()),
            data_access: None,
            interrupt: None,
        };
        fault.annotate(pc, &mut diag);
        diag
    }

    /// Fault-discards-all: restore the batch-entry state, drop staged
    /// stores, fetch runs and effects, rewind the per-step trace to
    /// the batch entry, and mark the unit faulted.
    ///
    /// Retirements discarded by the rollback never reach the trace
    /// stream: the rollback truncates their `PpuStateHash` /
    /// `PpuStateFull` entries, barrier records and code events, and restores
    /// `retirement_counter` and the issue model's scalar state. The caches
    /// and the branch predictor keep the changes of the discarded
    /// instructions.
    fn discard_batch(&mut self, entry: &BatchEntry, effects: &mut Vec<Effect>) {
        if let Some(snap) = entry.snapshot.as_ref() {
            self.state = snap.clone();
        }
        if let Some(core) = entry.timing {
            self.timing.restore(core);
        }
        self.store_buf.clear();
        self.drop_fetch_runs();
        effects.clear();
        self.per_step_hashes.truncate(entry.hashes);
        self.per_step_full_states.truncate(entry.fulls);
        self.barriers.truncate(entry.barriers);
        self.code_events.truncate(entry.code_events);
        self.retirement_counter = entry.retired;
        self.status = UnitStatus::Faulted;
    }

    /// Roll the batch back and yield `fault` at `pc`.
    ///
    /// It takes the diagnostics before the rollback, which erases the
    /// fault site's registers.
    pub(super) fn fault_yield(
        &mut self,
        entry: &BatchEntry,
        effects: &mut Vec<Effect>,
        pc: u64,
        fault: &PpuFault,
    ) -> ExecutionStepResult {
        let diag = self.fault_diag(pc, fault);
        self.discard_batch(entry, effects);
        ExecutionStepResult {
            yield_reason: YieldReason::Fault,
            consumed_cost: InstructionCost::ZERO,
            local_diagnostics: diag,
            fault: Some(FaultKind::Guest(fault.guest_code())),
            syscall_args: None,
        }
    }
}
