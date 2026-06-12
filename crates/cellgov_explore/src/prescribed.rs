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

    /// Force `choice` on the first scheduling decision, then fall
    /// back to round-robin. Suits the snapshot/restore path where
    /// the host runtime's step counter is already at the branch
    /// point and only one override is needed.
    pub fn single_choice(choice: UnitId) -> Self {
        Self::new(vec![Some(choice)])
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
#[path = "tests/prescribed_tests.rs"]
mod tests;
