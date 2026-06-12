//! Shared fixtures for sibling test modules.

#![cfg(test)]

use cellgov_effects::Effect;
use cellgov_event::UnitId;
use cellgov_exec::{
    ExecutionContext, ExecutionStepResult, ExecutionUnit, LocalDiagnostics, UnitStatus, YieldReason,
};
use cellgov_time::{Budget, InstructionCost};

#[derive(Clone)]
pub(super) struct CountingUnit {
    pub(super) id: UnitId,
    pub(super) steps: u64,
}

impl ExecutionUnit for CountingUnit {
    type Snapshot = u64;

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
        self.steps += 1;
        effects.push(Effect::TraceMarker {
            marker: self.steps as u32,
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

    fn snapshot(&self) -> u64 {
        self.steps
    }
}

#[derive(Clone)]
pub(super) struct LyingUnit;

impl ExecutionUnit for LyingUnit {
    type Snapshot = ();

    fn unit_id(&self) -> UnitId {
        UnitId::new(999)
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
            yield_reason: YieldReason::Finished,
            consumed_cost: InstructionCost::new(budget.raw()),
            local_diagnostics: LocalDiagnostics::empty(),
            fault: None,
            syscall_args: None,
        }
    }

    fn snapshot(&self) {}
}

#[derive(Clone)]
pub(super) struct StatusUnit {
    pub(super) id: UnitId,
    pub(super) status: std::rc::Rc<std::cell::Cell<UnitStatus>>,
}

impl ExecutionUnit for StatusUnit {
    type Snapshot = ();
    fn unit_id(&self) -> UnitId {
        self.id
    }
    fn status(&self) -> UnitStatus {
        self.status.get()
    }
    fn run_until_yield(
        &mut self,
        budget: Budget,
        _ctx: &ExecutionContext<'_>,
        _effects: &mut Vec<Effect>,
    ) -> ExecutionStepResult {
        ExecutionStepResult {
            yield_reason: YieldReason::Finished,
            consumed_cost: InstructionCost::new(budget.raw()),
            local_diagnostics: LocalDiagnostics::empty(),
            fault: None,
            syscall_args: None,
        }
    }
    fn snapshot(&self) {}
}

pub(super) struct StatusHandle(pub(super) std::rc::Rc<std::cell::Cell<UnitStatus>>);

impl StatusHandle {
    pub(super) fn set(&self, s: UnitStatus) {
        self.0.set(s);
    }
}

pub(super) fn status_unit(s: UnitStatus) -> (StatusHandle, impl FnOnce(UnitId) -> StatusUnit) {
    let cell = std::rc::Rc::new(std::cell::Cell::new(s));
    let cell_for_factory = cell.clone();
    (StatusHandle(cell), move |id| StatusUnit {
        id,
        status: cell_for_factory,
    })
}
