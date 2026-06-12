//! Prescribed-scheduler override sequencing against the fallback selection order.

use super::*;

// Local stub: cellgov_testkit depends transitively on this crate's
// scheduler trait, so we cannot pull its fixtures in here.
use cellgov_core::UnitRegistry;
use cellgov_effects::Effect;
use cellgov_exec::{
    ExecutionContext, ExecutionStepResult, ExecutionUnit, LocalDiagnostics, YieldReason,
};
use cellgov_time::{Budget, InstructionCost};
use std::cell::Cell;

#[derive(Clone)]
struct StubUnit {
    id: UnitId,
    status: Cell<UnitStatus>,
}
impl StubUnit {
    fn new(id: UnitId) -> Self {
        Self {
            id,
            status: Cell::new(UnitStatus::Runnable),
        }
    }
}
impl ExecutionUnit for StubUnit {
    type Snapshot = ();
    fn unit_id(&self) -> UnitId {
        self.id
    }
    fn status(&self) -> UnitStatus {
        self.status.get()
    }
    fn run_until_yield(
        &mut self,
        b: Budget,
        _: &ExecutionContext<'_>,
        effects: &mut Vec<Effect>,
    ) -> ExecutionStepResult {
        effects.push(Effect::TraceMarker {
            marker: 0,
            source: self.id,
        });
        ExecutionStepResult {
            yield_reason: YieldReason::BudgetExhausted,
            consumed_cost: InstructionCost::new(b.raw()),
            local_diagnostics: LocalDiagnostics::empty(),
            fault: None,
            syscall_args: None,
        }
    }
    fn snapshot(&self) {}
}

#[test]
fn override_forces_specific_unit() {
    let mut r = UnitRegistry::new();
    r.register_with(StubUnit::new);
    r.register_with(StubUnit::new);

    let mut s = PrescribedScheduler::new(vec![Some(UnitId::new(1))]);
    assert_eq!(s.select_next(&r), Some(UnitId::new(1)));
    assert_eq!(s.select_next(&r), Some(UnitId::new(0)));
}

#[test]
fn none_override_defers_to_fallback() {
    let mut r = UnitRegistry::new();
    r.register_with(StubUnit::new);
    r.register_with(StubUnit::new);

    let mut s = PrescribedScheduler::new(vec![None, Some(UnitId::new(0))]);
    assert_eq!(s.select_next(&r), Some(UnitId::new(0)));
    assert_eq!(s.select_next(&r), Some(UnitId::new(0)));
}
