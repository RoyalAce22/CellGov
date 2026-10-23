//! A `TimeOverflow` refusal comes before the unit runs.

use cellgov_effects::Effect;
use cellgov_event::UnitId;
use cellgov_exec::{
    ExecutionContext, ExecutionStepResult, ExecutionUnit, LocalDiagnostics, UnitStatus, YieldReason,
};
use cellgov_mem::GuestMemory;
use cellgov_time::{Budget, GuestTicks, InstructionCost};

use crate::runtime::{Runtime, StepError};

/// Finishes on its second run, and charges its whole budget each run.
#[derive(Clone)]
struct TwoRunUnit {
    id: UnitId,
    runs: u64,
}

impl ExecutionUnit for TwoRunUnit {
    type Snapshot = u64;

    fn unit_id(&self) -> UnitId {
        self.id
    }

    fn status(&self) -> UnitStatus {
        if self.runs >= 2 {
            UnitStatus::Finished
        } else {
            UnitStatus::Runnable
        }
    }

    fn run_until_yield(
        &mut self,
        budget: Budget,
        _ctx: &ExecutionContext<'_>,
        _effects: &mut Vec<Effect>,
    ) -> ExecutionStepResult {
        self.runs += 1;
        ExecutionStepResult {
            yield_reason: YieldReason::BudgetExhausted,
            consumed_cost: InstructionCost::new(budget.raw()),
            local_diagnostics: LocalDiagnostics::empty(),
            fault: None,
            syscall_args: None,
        }
    }

    fn snapshot(&self) -> u64 {
        self.runs
    }
}

#[test]
fn a_refused_time_overflow_leaves_the_unit_and_its_inputs_unchanged() {
    let budget = u64::MAX - 5;
    let mut rt = Runtime::new(GuestMemory::new(0), Budget::new(budget), 100);
    let id = rt
        .registry_mut()
        .register_with(|id| TwoRunUnit { id, runs: 0 });
    let first = rt.step().expect("the first step fits under u64::MAX");
    assert_eq!(first.time_after, GuestTicks::new(budget));
    rt.registry_mut().set_syscall_return(id, 0x55);

    assert_eq!(rt.step().unwrap_err(), StepError::TimeOverflow);

    // A second run would have finished the unit.
    assert_eq!(
        rt.registry().effective_status(id),
        Some(UnitStatus::Runnable)
    );
    assert_eq!(rt.registry().clock(id), GuestTicks::new(budget));
    assert_eq!(rt.steps_taken(), 1);
    assert_eq!(rt.time(), GuestTicks::new(budget));
    assert_eq!(rt.registry_mut().drain_syscall_return(id), Some(0x55));
}
