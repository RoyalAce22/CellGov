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

// The stub units cannot express a syscall yield, so stickiness is
// exercised by driving notify_yielded on the scheduler pair directly.
#[test]
fn notify_yielded_reaches_fallback_stickiness() {
    let mut r = UnitRegistry::new();
    r.register_with(StubUnit::new);
    r.register_with(StubUnit::new);

    let mut baseline = cellgov_core::RoundRobinScheduler::new();
    let mut prescribed = PrescribedScheduler::new(vec![]);

    assert_eq!(baseline.select_next(&r), Some(UnitId::new(0)));
    assert_eq!(prescribed.select_next(&r), Some(UnitId::new(0)));

    // Non-waking syscall: round-robin re-selects the same unit.
    baseline.notify_yielded(UnitId::new(0), YieldReason::Syscall, false, false);
    prescribed.notify_yielded(UnitId::new(0), YieldReason::Syscall, false, false);

    assert_eq!(baseline.select_next(&r), Some(UnitId::new(0)));
    assert_eq!(
        prescribed.select_next(&r),
        Some(UnitId::new(0)),
        "exhausted prescription must reproduce the baseline sticky pick"
    );
}

fn three_runnable_units() -> UnitRegistry {
    let mut r = UnitRegistry::new();
    for _ in 0..3 {
        r.register_with(StubUnit::new);
    }
    r
}

#[test]
fn a_prescribed_prefix_of_the_baseline_continues_its_rotation_exactly() {
    let r = three_runnable_units();
    let mut baseline = cellgov_core::RoundRobinScheduler::new();
    let expected: Vec<Option<UnitId>> = (0..8).map(|_| baseline.select_next(&r)).collect();

    let prefix = expected[..4].to_vec();
    let mut prescribed = PrescribedScheduler::new(prefix);
    let actual: Vec<Option<UnitId>> = (0..8).map(|_| prescribed.select_next(&r)).collect();
    assert_eq!(actual, expected);
}

#[test]
fn the_rotation_resumes_after_the_overridden_unit() {
    let r = three_runnable_units();
    let mut s = PrescribedScheduler::new(vec![Some(UnitId::new(2))]);
    let picks: Vec<Option<UnitId>> = (0..4).map(|_| s.select_next(&r)).collect();
    assert_eq!(
        picks,
        [2, 0, 1, 2].map(|i| Some(UnitId::new(i))),
        "the first fallback pick rotates past the forced unit, not from a stale cursor"
    );
}

#[test]
fn a_non_runnable_override_leaves_the_cursor_to_the_fallback_pick() {
    let mut r = three_runnable_units();
    r.set_status_override(UnitId::new(2), UnitStatus::Blocked);
    let mut s = PrescribedScheduler::new(vec![Some(UnitId::new(2))]);
    assert_eq!(s.select_next(&r), Some(UnitId::new(0)));
    assert_eq!(s.select_next(&r), Some(UnitId::new(1)));
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
