//! Scheduler that replays a recorded per-step choice list and falls
//! back to round-robin beyond the list or where the prescribed unit is
//! not runnable. It changes no runtime state; it decides which runnable
//! unit the runtime picks next.

use cellgov_core::{RoundRobinScheduler, Scheduler, UnitRegistry};
use cellgov_event::UnitId;
use cellgov_exec::{UnitStatus, YieldReason};

/// Scheduler that picks from a prescribed list, then falls back to
/// round-robin.
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

    /// Force `choice` on the first scheduling decision, then fall
    /// back to round-robin.
    pub fn single_choice(choice: UnitId) -> Self {
        Self::new(vec![Some(choice)])
    }
}

impl Scheduler for PrescribedScheduler {
    fn select_next(&mut self, registry: &UnitRegistry) -> Option<UnitId> {
        let override_for_step = self.overrides.get(self.step).copied().flatten();
        if let Some(uid) = override_for_step {
            if registry.effective_status(uid) == Some(UnitStatus::Runnable) {
                // The fallback's cursor follows the unit that ran (see
                // `RoundRobinScheduler::note_selected`).
                self.fallback.note_selected(uid);
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

    fn notify_yielded(
        &mut self,
        unit: UnitId,
        yield_reason: YieldReason,
        woke_others: bool,
        holds_critical_section: bool,
    ) {
        // The fallback reads every yield, under a prescription too, so
        // its stickiness state (see `RoundRobinScheduler`) matches the
        // baseline's when the prescription ends.
        self.fallback
            .notify_yielded(unit, yield_reason, woke_others, holds_critical_section);
    }
}

#[cfg(test)]
#[path = "tests/prescribed_tests.rs"]
mod tests;
