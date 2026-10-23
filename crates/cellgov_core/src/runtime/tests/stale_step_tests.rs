//! A commit closes only the uncommitted step the runtime ran last.

use cellgov_effects::Effect;
use cellgov_event::UnitId;
use cellgov_exec::{
    ExecutionContext, ExecutionStepResult, ExecutionUnit, LocalDiagnostics, UnitStatus, YieldReason,
};
use cellgov_mem::GuestMemory;
use cellgov_time::{Budget, InstructionCost};

use crate::commit::CommitError;
use crate::runtime::{Runtime, StepError};
use crate::scheduler::RoundRobinScheduler;

/// Always runnable; marks every step with one trace marker.
#[derive(Clone)]
struct Marker {
    id: UnitId,
}

impl ExecutionUnit for Marker {
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
        effects: &mut Vec<Effect>,
    ) -> ExecutionStepResult {
        effects.push(Effect::TraceMarker {
            marker: 1,
            source: self.id,
        });
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

fn build() -> Runtime {
    let mut rt = Runtime::new(GuestMemory::new(16), Budget::new(1), 100);
    rt.register_unit_with(|id| Marker { id });
    rt.register_unit_with(|id| Marker { id });
    rt
}

#[test]
fn a_step_another_step_replaced_is_refused_and_left_whole() {
    let mut rt = build();
    let mut first = rt.step().expect("both units are runnable");
    let mut second = rt.step().expect("both units are runnable");
    assert_ne!(first.token.unit(), second.token.unit());
    let epoch = rt.epoch();

    assert_eq!(rt.commit_step(&mut first), Err(CommitError::StaleStep));
    assert_eq!(rt.epoch(), epoch, "a refused commit is no commit boundary");
    assert_eq!(first.effects.len(), 1, "the refusal leaves the step whole");

    rt.commit_step(&mut second).expect("the last step commits");
    assert!(second.effects.is_empty());
}

#[test]
fn a_committed_step_is_refused_a_second_time() {
    let mut rt = build();
    let mut step = rt.step().expect("both units are runnable");
    rt.commit_step(&mut step).expect("the step commits");
    let epoch = rt.epoch();

    assert_eq!(rt.commit_step(&mut step), Err(CommitError::StaleStep));
    assert_eq!(rt.epoch(), epoch);
}

#[test]
fn a_restore_leaves_no_step_to_commit() {
    let mut rt = build();
    let snap = rt.snapshot();
    let mut step = rt.step().expect("both units are runnable");
    rt.restore_into(&snap);

    assert_eq!(rt.commit_step(&mut step), Err(CommitError::StaleStep));
}

#[test]
fn a_step_from_before_a_restore_never_matches_one_after_it() {
    let mut rt = build();
    let snap = rt.snapshot();
    let mut before = rt.step().expect("both units are runnable");
    rt.restore_into(&snap);
    rt.set_scheduler(RoundRobinScheduler::new());
    let mut after = rt.step().expect("both units are runnable");
    assert_eq!(
        (before.token.index(), before.token.unit()),
        (after.token.index(), after.token.unit()),
        "the replay takes the same step"
    );

    assert_eq!(rt.commit_step(&mut before), Err(CommitError::StaleStep));
    rt.commit_step(&mut after)
        .expect("the step after the restore commits");
}

#[test]
fn a_refused_step_closes_the_step_before_it() {
    let mut rt = Runtime::new(GuestMemory::new(16), Budget::new(1), 1);
    rt.register_unit_with(|id| Marker { id });
    let mut step = rt.step().expect("the unit is runnable");
    assert_eq!(rt.step(), Err(StepError::MaxStepsExceeded));

    assert_eq!(rt.commit_step(&mut step), Err(CommitError::StaleStep));
}

#[test]
fn an_unchecked_commit_closes_the_step() {
    let mut rt = build();
    let mut step = rt.step().expect("both units are runnable");
    rt.commit_unchecked(&step.result, &step.effects)
        .expect("the step commits");

    assert_eq!(rt.commit_step(&mut step), Err(CommitError::StaleStep));
}
