//! Scheduler that replays a recorded per-step choice list and falls
//! back to the runtime's lowest-clock rule beyond the list or where the
//! prescribed unit is not runnable. It changes no runtime state; it
//! decides which runnable unit the runtime picks next.

use cellgov_core::{LowestClockScheduler, Scheduler, UnitRegistry};
use cellgov_event::UnitId;
use cellgov_exec::UnitStatus;

/// Scheduler that picks from a prescribed list, then falls back to the
/// lowest-clock rule.
pub struct PrescribedScheduler {
    overrides: Vec<Option<UnitId>>,
    step: usize,
    fallback: LowestClockScheduler,
}

impl PrescribedScheduler {
    /// Create a scheduler with per-step overrides; `None` at index `i`
    /// defers step `i` to the lowest-clock fallback.
    pub fn new(overrides: Vec<Option<UnitId>>) -> Self {
        Self {
            overrides,
            step: 0,
            fallback: LowestClockScheduler::new(),
        }
    }

    /// Force `choice` on the first scheduling decision, then fall
    /// back to the lowest-clock rule.
    pub fn single_choice(choice: UnitId) -> Self {
        Self::new(vec![Some(choice)])
    }
}

impl Scheduler for PrescribedScheduler {
    fn select_next(&mut self, registry: &UnitRegistry) -> Option<UnitId> {
        let override_for_step = self.overrides.get(self.step).copied().flatten();
        if let Some(uid) = override_for_step {
            if registry.effective_status(uid) == Some(UnitStatus::Runnable) {
                self.step += 1;
                return Some(uid);
            }
        }
        let picked = self.fallback.select_next(registry);
        // `Runtime::step` asks again after a time warp. The cursor
        // indexes the steps the runtime took, so an unanswered call
        // leaves it where it is.
        if picked.is_some() {
            self.step += 1;
        }
        picked
    }
}

#[cfg(test)]
#[path = "tests/prescribed_tests.rs"]
mod tests;
