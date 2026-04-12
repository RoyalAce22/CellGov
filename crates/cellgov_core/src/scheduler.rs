//! Deterministic scheduler -- step 1 of the runtime pipeline.
//!
//! The runtime pipeline is:
//!
//! 1. select runnable unit deterministically
//! 2. grant budget
//! 3. run unit until yield
//! 4. ... (validation, commit, event injection, time advance, trace)
//!
//! This module owns step 1. Given a [`UnitRegistry`], it picks the next
//! [`UnitId`] to schedule based purely on the unit statuses currently
//! reported by the registry. It does not call `run_until_yield`, it
//! does not touch the commit pipeline, it does not advance time. The
//! runtime loop in [`crate::runtime`] will compose the scheduler with
//! the rest of the pipeline once the pipeline exists.
//!
//! Determinism contract: a scheduler implementation is a pure function
//! of `(its own state, registry contents)`. It must not consult host
//! time, host thread scheduling, `HashMap` iteration order, or any
//! other nondeterministic input.

use crate::registry::UnitRegistry;
use cellgov_event::UnitId;
use cellgov_exec::UnitStatus;

/// Pluggable scheduling policy.
///
/// Concrete scheduler types stay private to `cellgov_core`; other
/// crates see only traits and immutable data packets. This trait is
/// the public seam other crates use; the concrete
/// [`RoundRobinScheduler`] is currently the only implementation
/// behind it.
pub trait Scheduler {
    /// Select the next runnable unit, or `None` if no unit is
    /// currently runnable.
    ///
    /// Implementations may mutate internal scheduler state (a cursor,
    /// a fairness counter, etc.) but must not mutate the registry.
    /// They must be deterministic: identical scheduler state plus an
    /// identical sequence of registry-status snapshots must produce
    /// an identical sequence of selections.
    fn select_next(&mut self, registry: &UnitRegistry) -> Option<UnitId>;
}

/// A round-robin scheduler.
///
/// Walks the registry in id order, starting from the position after
/// the last selection, and returns the first unit it finds whose
/// [`UnitStatus`] is `Runnable`. Wraps around. Skips `Blocked`,
/// `Faulted`, and `Finished` units.
///
/// This is the simplest deterministic scheduler that guarantees no
/// runnable unit can starve under fixed workload: every runnable unit
/// gets a turn before any unit gets a second turn (modulo blocking
/// transitions).
#[derive(Debug, Default)]
pub struct RoundRobinScheduler {
    /// The id of the most recently selected unit, used as the cursor
    /// for the next call. `None` means no selection yet -- the next
    /// call starts at the beginning of the registry.
    last_scheduled: Option<UnitId>,
}

impl RoundRobinScheduler {
    /// Construct a fresh scheduler with no prior selection.
    #[inline]
    pub fn new() -> Self {
        Self::default()
    }

    /// Return the id of the most recently selected unit, if any. Used
    /// by tests and trace tooling to inspect the cursor.
    #[inline]
    pub fn last_scheduled(&self) -> Option<UnitId> {
        self.last_scheduled
    }
}

impl Scheduler for RoundRobinScheduler {
    fn select_next(&mut self, registry: &UnitRegistry) -> Option<UnitId> {
        if registry.is_empty() {
            return None;
        }
        // Two-pass scan over the registry in id order:
        //
        // First pass: every unit strictly after `last_scheduled`.
        // Second pass: every unit from the beginning up to and
        // including `last_scheduled` (so we can re-pick the same unit
        // if it is the only runnable one and was the last selection).
        //
        // This is structurally equivalent to "rotate the registry so
        // last_scheduled is at the end, then return the first runnable
        // unit". Doing it as two filtered iterations avoids allocation.
        let cursor = self.last_scheduled;

        let after_cursor = registry
            .iter()
            .filter(|(id, _)| match cursor {
                Some(c) => *id > c,
                None => true,
            })
            .find(|(id, _)| registry.effective_status(*id) == Some(UnitStatus::Runnable));

        let chosen = if let Some((id, _)) = after_cursor {
            Some(id)
        } else {
            // Wrap: try from the beginning, including the cursor itself.
            registry
                .iter()
                .filter(|(id, _)| match cursor {
                    Some(c) => *id <= c,
                    None => false,
                })
                .find(|(id, _)| registry.effective_status(*id) == Some(UnitStatus::Runnable))
                .map(|(id, _)| id)
        };

        if let Some(id) = chosen {
            self.last_scheduled = Some(id);
        }
        chosen
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cellgov_effects::Effect;
    use cellgov_exec::{
        ExecutionContext, ExecutionStepResult, ExecutionUnit, LocalDiagnostics, YieldReason,
    };
    use cellgov_time::Budget;
    use std::cell::Cell;

    // Local test doubles -- cellgov_testkit depends on cellgov_core,
    // so a reverse dev-dependency would create a cycle.

    /// A test unit whose status is configurable per-test. Uses interior
    /// mutability so tests can flip the status without re-registering.
    struct TestUnit {
        id: UnitId,
        status: Cell<UnitStatus>,
    }

    impl TestUnit {
        fn new(id: UnitId, status: UnitStatus) -> Self {
            Self {
                id,
                status: Cell::new(status),
            }
        }
    }

    impl ExecutionUnit for TestUnit {
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
        ) -> ExecutionStepResult {
            ExecutionStepResult {
                yield_reason: YieldReason::BudgetExhausted,
                consumed_budget: budget,
                emitted_effects: vec![Effect::TraceMarker {
                    marker: 0,
                    source: self.id,
                }],
                local_diagnostics: LocalDiagnostics::empty(),
                fault: None,
                syscall_args: None,
            }
        }

        fn snapshot(&self) {}
    }

    fn registry_with(statuses: &[UnitStatus]) -> UnitRegistry {
        let mut r = UnitRegistry::new();
        for &s in statuses {
            r.register_with(|id| TestUnit::new(id, s));
        }
        r
    }

    #[test]
    fn empty_registry_yields_none() {
        let mut s = RoundRobinScheduler::new();
        let r = UnitRegistry::new();
        assert_eq!(s.select_next(&r), None);
    }

    #[test]
    fn all_blocked_yields_none() {
        let mut s = RoundRobinScheduler::new();
        let r = registry_with(&[UnitStatus::Blocked, UnitStatus::Blocked]);
        assert_eq!(s.select_next(&r), None);
    }

    #[test]
    fn single_runnable_picks_it_repeatedly() {
        let mut s = RoundRobinScheduler::new();
        let r = registry_with(&[UnitStatus::Runnable]);
        assert_eq!(s.select_next(&r), Some(UnitId::new(0)));
        assert_eq!(s.select_next(&r), Some(UnitId::new(0)));
        assert_eq!(s.select_next(&r), Some(UnitId::new(0)));
    }

    #[test]
    fn round_robin_visits_each_runnable_in_id_order() {
        let mut s = RoundRobinScheduler::new();
        let r = registry_with(&[
            UnitStatus::Runnable,
            UnitStatus::Runnable,
            UnitStatus::Runnable,
        ]);
        assert_eq!(s.select_next(&r), Some(UnitId::new(0)));
        assert_eq!(s.select_next(&r), Some(UnitId::new(1)));
        assert_eq!(s.select_next(&r), Some(UnitId::new(2)));
        // Wraps.
        assert_eq!(s.select_next(&r), Some(UnitId::new(0)));
        assert_eq!(s.select_next(&r), Some(UnitId::new(1)));
    }

    #[test]
    fn skips_blocked_faulted_finished() {
        let mut s = RoundRobinScheduler::new();
        let r = registry_with(&[
            UnitStatus::Blocked,
            UnitStatus::Runnable,
            UnitStatus::Faulted,
            UnitStatus::Runnable,
            UnitStatus::Finished,
        ]);
        // First call from origin: skip 0 (blocked), pick 1.
        assert_eq!(s.select_next(&r), Some(UnitId::new(1)));
        // Next call from after 1: skip 2 (faulted), pick 3.
        assert_eq!(s.select_next(&r), Some(UnitId::new(3)));
        // Next call from after 3: skip 4 (finished), wrap, skip 0,
        // and pick 1 again.
        assert_eq!(s.select_next(&r), Some(UnitId::new(1)));
    }

    #[test]
    fn round_robin_with_only_one_runnable_among_many() {
        // Five units, only the third is runnable. Every call returns it.
        let r = registry_with(&[
            UnitStatus::Blocked,
            UnitStatus::Blocked,
            UnitStatus::Runnable,
            UnitStatus::Blocked,
            UnitStatus::Blocked,
        ]);
        let mut s = RoundRobinScheduler::new();
        for _ in 0..5 {
            assert_eq!(s.select_next(&r), Some(UnitId::new(2)));
        }
    }

    #[test]
    fn last_scheduled_tracks_cursor() {
        let r = registry_with(&[UnitStatus::Runnable, UnitStatus::Runnable]);
        let mut s = RoundRobinScheduler::new();
        assert_eq!(s.last_scheduled(), None);
        let _ = s.select_next(&r);
        assert_eq!(s.last_scheduled(), Some(UnitId::new(0)));
        let _ = s.select_next(&r);
        assert_eq!(s.last_scheduled(), Some(UnitId::new(1)));
    }

    #[test]
    fn status_override_blocks_a_runnable_unit() {
        // Unit 0 self-reports Runnable, but the runtime overrides it
        // to Blocked. The scheduler must skip it.
        let mut r = registry_with(&[UnitStatus::Runnable, UnitStatus::Runnable]);
        r.set_status_override(UnitId::new(0), UnitStatus::Blocked);
        let mut s = RoundRobinScheduler::new();
        // Every call should return unit 1 since unit 0 is overridden.
        assert_eq!(s.select_next(&r), Some(UnitId::new(1)));
        assert_eq!(s.select_next(&r), Some(UnitId::new(1)));
        // Clear the override -- unit 0 is runnable again.
        r.clear_status_override(UnitId::new(0));
        assert_eq!(s.select_next(&r), Some(UnitId::new(0)));
    }
}
