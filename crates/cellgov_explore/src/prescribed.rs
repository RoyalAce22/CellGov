//! Scheduler that replays a recorded per-step choice list, falling
//! back to round-robin beyond the list or when the prescribed unit
//! is not currently runnable.
//!
//! Observation-only: installing a `PrescribedScheduler` never mutates
//! the runtime's state; it only biases which runnable unit the
//! runtime picks next.

use cellgov_core::{RoundRobinScheduler, Scheduler, UnitRegistry};
use cellgov_event::UnitId;
use cellgov_exec::UnitStatus;

/// Scheduler that picks from a prescribed list, then falls back to
/// round-robin.
///
/// At step `i`, if `overrides[i] == Some(uid)` and `uid` is runnable,
/// `uid` is chosen. Otherwise the fallback picks.
pub struct PrescribedScheduler {
    overrides: Vec<Option<UnitId>>,
    step: usize,
    fallback: RoundRobinScheduler,
}

impl PrescribedScheduler {
    /// Create a scheduler with per-step overrides; `None` at index `i`
    /// defers step `i` to the round-robin fallback.
    pub fn new(overrides: Vec<Option<UnitId>>) -> Self {
        Self {
            overrides,
            step: 0,
            fallback: RoundRobinScheduler::new(),
        }
    }
}

impl Scheduler for PrescribedScheduler {
    fn select_next(&mut self, registry: &UnitRegistry) -> Option<UnitId> {
        let choice = if self.step < self.overrides.len() {
            if let Some(uid) = self.overrides[self.step] {
                if registry.effective_status(uid) == Some(UnitStatus::Runnable) {
                    self.step += 1;
                    return Some(uid);
                }
            }
            None
        } else {
            None
        };
        self.step += 1;
        if choice.is_some() {
            return choice;
        }
        self.fallback.select_next(registry)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Local stub: cellgov_testkit depends transitively on this crate's
    // scheduler trait, so we cannot pull its fixtures in here.
    use cellgov_core::UnitRegistry;
    use cellgov_effects::Effect;
    use cellgov_exec::{
        ExecutionContext, ExecutionStepResult, ExecutionUnit, LocalDiagnostics, YieldReason,
    };
    use cellgov_time::Budget;
    use std::cell::Cell;

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
                consumed_budget: b,
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
}
