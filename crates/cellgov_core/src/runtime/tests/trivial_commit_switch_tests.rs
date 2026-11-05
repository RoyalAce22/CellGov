//! The host switch over the trivial-step commit fast path.

use cellgov_event::UnitId;
use cellgov_exec::{ExecutionStepResult, LocalDiagnostics, YieldReason};
use cellgov_mem::GuestMemory;
use cellgov_time::{Budget, InstructionCost};

use crate::runtime::state::Runtime;
use crate::runtime::types::RuntimeMode;

/// A step that retired its budget and left nothing to commit.
fn trivial_step() -> ExecutionStepResult {
    ExecutionStepResult {
        yield_reason: YieldReason::BudgetExhausted,
        consumed_cost: InstructionCost::new(1),
        local_diagnostics: LocalDiagnostics::empty(),
        fault: None,
        syscall_args: None,
    }
}

fn fault_driven() -> Runtime {
    let mut rt = Runtime::new(GuestMemory::new(64), Budget::new(1), 1);
    rt.set_mode(RuntimeMode::FaultDriven);
    rt
}

#[test]
fn a_trivial_step_takes_the_fast_path_by_default() {
    let rt = fault_driven();
    assert!(rt.trivial_commit_eligible(UnitId::new(0), &trivial_step(), &[]));
}

#[test]
fn a_forbidden_fast_path_sends_a_trivial_step_through_the_full_commit() {
    let mut rt = fault_driven();
    rt.set_trivial_commit_fast_path(false);
    assert!(!rt.trivial_commit_eligible(UnitId::new(0), &trivial_step(), &[]));
}
