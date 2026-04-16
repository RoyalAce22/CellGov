//! A scheduler that follows a prescribed sequence of unit choices,
//! falling back to round-robin for steps beyond the sequence.
//!
//! Used by the explorer to replay a scenario with an alternate
//! scheduling decision at a specific branching point.

use cellgov_core::{RoundRobinScheduler, Scheduler, UnitRegistry};
use cellgov_event::UnitId;
use cellgov_exec::UnitStatus;

/// Scheduler that picks from a prescribed list, then falls back to
/// round-robin. At step `i`, if `overrides[i]` is set and the unit
/// is runnable, that unit is chosen. Otherwise the fallback picks.
pub struct PrescribedScheduler {
    overrides: Vec<Option<UnitId>>,
    step: usize,
    fallback: RoundRobinScheduler,
}

impl PrescribedScheduler {
    /// Create a scheduler with the given per-step overrides.
    /// `overrides[i] = Some(uid)` forces unit `uid` at step `i`;
    /// `None` defers to round-robin.
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
            // Override not applicable; fall through.
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

    // Minimal test unit for scheduler tests. We cannot import from
    // cellgov_testkit (it depends on cellgov_core which depends on
    // the scheduler), so we use a local double.
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
        r.register_with(StubUnit::new); // 0
        r.register_with(StubUnit::new); // 1

        // Force unit 1 at step 0 (round-robin would pick 0).
        let mut s = PrescribedScheduler::new(vec![Some(UnitId::new(1))]);
        assert_eq!(s.select_next(&r), Some(UnitId::new(1)));
        // Step 1: no override, falls back to round-robin.
        assert_eq!(s.select_next(&r), Some(UnitId::new(0)));
    }

    #[test]
    fn none_override_defers_to_fallback() {
        let mut r = UnitRegistry::new();
        r.register_with(StubUnit::new);
        r.register_with(StubUnit::new);

        let mut s = PrescribedScheduler::new(vec![None, Some(UnitId::new(0))]);
        // Step 0: None -> fallback picks unit 0.
        assert_eq!(s.select_next(&r), Some(UnitId::new(0)));
        // Step 1: override picks unit 0.
        assert_eq!(s.select_next(&r), Some(UnitId::new(0)));
    }
}
