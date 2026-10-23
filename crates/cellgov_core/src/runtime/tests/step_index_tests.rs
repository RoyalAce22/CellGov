//! The observations carry the index of the step they belong to.

use cellgov_effects::Effect;
use cellgov_event::UnitId;
use cellgov_exec::{
    ExecutionContext, ExecutionStepResult, ExecutionUnit, LocalDiagnostics, UnitStatus, YieldReason,
};
use cellgov_mem::GuestMemory;
use cellgov_time::{Budget, InstructionCost};

use crate::runtime::{Runtime, StepError, StepIndex};

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
fn each_step_and_its_observations_carry_one_index() {
    let mut rt = Runtime::new(GuestMemory::new(16), Budget::new(1), 2);
    rt.register_unit_with(|id| Spinner { id });
    let start = rt.snapshot();

    for expected in 0..2 {
        let mut step = rt.step().expect("the unit is runnable");
        assert_eq!(step.token.index(), StepIndex::new(expected));
        assert_eq!(rt.observations().step, step.token.index());
        rt.commit_step(&mut step).expect("the step commits");
        assert_eq!(rt.observations().step, step.token.index());
    }

    assert_eq!(rt.step(), Err(StepError::MaxStepsExceeded));
    assert_eq!(
        rt.observations().step,
        StepIndex::new(2),
        "a refused step takes the index the next step would"
    );

    rt.restore_into(&start);
    assert_eq!(rt.observations().step, StepIndex::new(0));
}
