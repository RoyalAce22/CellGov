//! The step result every fixture returns.

use cellgov_exec::{ExecutionStepResult, LocalDiagnostics, YieldReason};
use cellgov_time::{Budget, InstructionCost};

/// A step that yields for `reason` after it spends the whole `budget`,
/// with no diagnostics, fault or syscall.
pub(crate) fn step(reason: YieldReason, budget: Budget) -> ExecutionStepResult {
    ExecutionStepResult {
        yield_reason: reason,
        consumed_cost: InstructionCost::new(budget.raw()),
        local_diagnostics: LocalDiagnostics::empty(),
        fault: None,
        syscall_args: None,
    }
}
