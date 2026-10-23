//! The runnable set a step returns travels in one reused buffer.

use cellgov_effects::Effect;
use cellgov_event::UnitId;
use cellgov_exec::{
    ExecutionContext, ExecutionStepResult, ExecutionUnit, LocalDiagnostics, UnitStatus, YieldReason,
};
use cellgov_mem::GuestMemory;
use cellgov_time::{Budget, InstructionCost};

use crate::runtime::Runtime;

/// Always runnable; charges its whole budget.
#[derive(Clone)]
struct Spinner {
    id: UnitId,
}

impl ExecutionUnit for Spinner {
    type Snapshot = ();

    fn unit_id(&self) -> UnitId {
        self.id
    }

    fn status(&self) -> UnitStatus {
        UnitStatus::Runnable
    }

    fn run_until_yield(
        &mut self,
        budget: Budget,
        _ctx: &ExecutionContext<'_>,
        _effects: &mut Vec<Effect>,
    ) -> ExecutionStepResult {
        ExecutionStepResult {
            yield_reason: YieldReason::BudgetExhausted,
            consumed_cost: InstructionCost::new(budget.raw()),
            local_diagnostics: LocalDiagnostics::empty(),
            fault: None,
            syscall_args: None,
        }
    }

    fn snapshot(&self) {}
}

#[test]
fn a_recycling_commit_hands_the_runnable_buffer_to_the_next_step() {
    let mut rt = Runtime::new(GuestMemory::new(16), Budget::new(1), 100);
    let a = rt.register_unit_with(|id| Spinner { id });
    let b = rt.register_unit_with(|id| Spinner { id });

    let mut first = rt.step().expect("both units are runnable");
    assert_eq!(first.runnable, vec![a, b]);
    let capacity = first.runnable.capacity();
    rt.commit_step(&mut first).expect("the step commits");
    assert!(first.runnable.is_empty());
    assert_eq!(rt.runnable_buf_capacity_for_tests(), capacity);

    let second = rt.step().expect("both units are runnable");
    assert_eq!(second.runnable, vec![a, b]);
    assert_eq!(second.runnable.capacity(), capacity);
}
