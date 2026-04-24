//! Deterministic scheduler: picks the next runnable [`UnitId`] from a
//! [`UnitRegistry`]. Does not run the unit; the runtime loop composes
//! the selection with commit, trace, and time advance.
//!
//! A scheduler implementation is a pure function of (its own state,
//! registry contents). No host time, no `HashMap` iteration order,
//! no host-thread scheduling input.

use crate::registry::UnitRegistry;
use cellgov_event::UnitId;
use cellgov_exec::UnitStatus;

/// Pluggable scheduling policy.
pub trait Scheduler {
    /// Select the next runnable unit, or `None` if none is runnable.
    ///
    /// May mutate scheduler-internal state; must not mutate the registry.
    /// Must be a deterministic function of scheduler state plus the
    /// sequence of registry-status snapshots observed.
    fn select_next(&mut self, registry: &UnitRegistry) -> Option<UnitId>;
}

/// Round-robin scheduler: walks the registry in id order from the
/// position after `last_scheduled`, returns the first `Runnable` unit,
/// wraps around. Skips `Blocked`, `Faulted`, `Finished`.
///
/// Relies on two [`UnitRegistry`] contracts:
///
/// 1. `registry.iter()` yields ids in ascending order; the two-pass
///    `id > cursor` / `id <= cursor` scan depends on it. A HashMap-
///    backed registry would silently reorder selections.
/// 2. `UnitId`s are monotonic and stable; recycling ids would point
///    the cursor at a different unit than the one it was set from.
#[derive(Debug, Default)]
pub struct RoundRobinScheduler {
    /// Cursor: id of the most recently selected unit; `None` at start.
    last_scheduled: Option<UnitId>,
}

impl RoundRobinScheduler {
    /// Construct a fresh scheduler.
    #[inline]
    pub fn new() -> Self {
        Self::default()
    }

    /// Id of the most recently selected unit, if any.
    ///
    /// Only meaningful while the registry's id-stability contract
    /// holds; unit removal would leave the cursor dangling. The
    /// disappearance case is debug-asserted on the next `select_next`;
    /// id recycling is not detected at this layer.
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
        if let Some(c) = self.last_scheduled {
            debug_assert!(
                registry.get(c).is_some(),
                "scheduler cursor {c:?} names an id not present in the registry \
                 (does not detect id recycling, only disappearance)"
            );
        }
        // Snapshot-once: reading `effective_status` twice could
        // diverge if a future refactor makes it stateful.
        let runnables: Vec<UnitId> = registry
            .iter()
            .filter(|(id, _)| registry.effective_status(*id) == Some(UnitStatus::Runnable))
            .map(|(id, _)| id)
            .collect();

        // Ascending order is the invariant the two-pass scan below
        // relies on; bounded size catches a runaway registry.
        debug_assert!(
            runnables.windows(2).all(|w| w[0] < w[1]),
            "scheduler runnables snapshot is not ascending: {runnables:?}"
        );
        debug_assert!(
            runnables.len() < 65_536,
            "scheduler runnables snapshot exceeded 65536; registry is likely broken"
        );

        let chosen = match runnables.len() {
            0 => None,
            1 => Some(runnables[0]),
            _ => match self.last_scheduled {
                Some(c) => runnables
                    .iter()
                    .copied()
                    .find(|&id| id > c)
                    .or_else(|| runnables.iter().copied().find(|&id| id <= c)),
                None => Some(runnables[0]),
            },
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
            effects: &mut Vec<Effect>,
        ) -> ExecutionStepResult {
            effects.push(Effect::TraceMarker {
                marker: 0,
                source: self.id,
            });
            ExecutionStepResult {
                yield_reason: YieldReason::BudgetExhausted,
                consumed_budget: budget,
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
    fn rotation_continues_correctly_when_cursor_unit_becomes_blocked() {
        let mut r = registry_with(&[
            UnitStatus::Runnable,
            UnitStatus::Runnable,
            UnitStatus::Runnable,
        ]);
        let mut s = RoundRobinScheduler::new();
        assert_eq!(s.select_next(&r), Some(UnitId::new(0)));
        assert_eq!(s.select_next(&r), Some(UnitId::new(1)));
        assert_eq!(s.last_scheduled(), Some(UnitId::new(1)));
        r.set_status_override(UnitId::new(1), UnitStatus::Blocked);
        assert_eq!(s.select_next(&r), Some(UnitId::new(2)));
        r.clear_status_override(UnitId::new(1));
        assert_eq!(s.select_next(&r), Some(UnitId::new(0)));
        assert_eq!(s.select_next(&r), Some(UnitId::new(1)));
    }

    #[test]
    fn all_blocked_with_cursor_set_yields_none_and_preserves_cursor() {
        let mut r = registry_with(&[UnitStatus::Runnable, UnitStatus::Runnable]);
        let mut s = RoundRobinScheduler::new();
        assert_eq!(s.select_next(&r), Some(UnitId::new(0)));
        assert_eq!(s.last_scheduled(), Some(UnitId::new(0)));
        // Now block everyone via override.
        r.set_status_override(UnitId::new(0), UnitStatus::Blocked);
        r.set_status_override(UnitId::new(1), UnitStatus::Blocked);
        assert_eq!(s.select_next(&r), None);
        assert_eq!(
            s.last_scheduled(),
            Some(UnitId::new(0)),
            "cursor must survive an all-blocked call so rotation resumes \
             correctly once units unblock"
        );
        r.clear_status_override(UnitId::new(1));
        assert_eq!(s.select_next(&r), Some(UnitId::new(1)));
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
        assert_eq!(s.select_next(&r), Some(UnitId::new(1)));
        assert_eq!(s.select_next(&r), Some(UnitId::new(3)));
        assert_eq!(s.select_next(&r), Some(UnitId::new(1)));
    }

    #[test]
    fn round_robin_with_only_one_runnable_among_many() {
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
    fn three_runnable_units_produce_identical_selection_sequence_across_runs() {
        let r_a = registry_with(&[
            UnitStatus::Runnable,
            UnitStatus::Runnable,
            UnitStatus::Runnable,
        ]);
        let r_b = registry_with(&[
            UnitStatus::Runnable,
            UnitStatus::Runnable,
            UnitStatus::Runnable,
        ]);
        let mut s_a = RoundRobinScheduler::new();
        let mut s_b = RoundRobinScheduler::new();
        let seq_a: Vec<_> = (0..100)
            .map(|_| s_a.select_next(&r_a).unwrap().raw())
            .collect();
        let seq_b: Vec<_> = (0..100)
            .map(|_| s_b.select_next(&r_b).unwrap().raw())
            .collect();
        assert_eq!(seq_a, seq_b);
        for (i, id) in seq_a.iter().enumerate() {
            assert_eq!(*id, (i % 3) as u64);
        }
    }

    #[test]
    fn single_runnable_fast_path_picks_it_in_multi_unit_registry() {
        let r = registry_with(&[
            UnitStatus::Blocked,
            UnitStatus::Runnable,
            UnitStatus::Blocked,
        ]);
        let mut s = RoundRobinScheduler::new();
        assert_eq!(s.select_next(&r), Some(UnitId::new(1)));
        assert_eq!(s.select_next(&r), Some(UnitId::new(1)));
        assert_eq!(s.last_scheduled(), Some(UnitId::new(1)));
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
        let mut r = registry_with(&[UnitStatus::Runnable, UnitStatus::Runnable]);
        r.set_status_override(UnitId::new(0), UnitStatus::Blocked);
        let mut s = RoundRobinScheduler::new();
        assert_eq!(s.select_next(&r), Some(UnitId::new(1)));
        assert_eq!(s.select_next(&r), Some(UnitId::new(1)));
        r.clear_status_override(UnitId::new(0));
        assert_eq!(s.select_next(&r), Some(UnitId::new(0)));
    }

    #[test]
    fn status_override_wakes_a_blocked_unit() {
        let mut r = registry_with(&[UnitStatus::Blocked, UnitStatus::Blocked]);
        let mut s = RoundRobinScheduler::new();
        assert_eq!(s.select_next(&r), None);
        r.set_status_override(UnitId::new(1), UnitStatus::Runnable);
        assert_eq!(s.select_next(&r), Some(UnitId::new(1)));
        r.set_status_override(UnitId::new(0), UnitStatus::Runnable);
        assert_eq!(s.select_next(&r), Some(UnitId::new(0)));
        assert_eq!(s.select_next(&r), Some(UnitId::new(1)));
    }

    #[test]
    fn cursor_advanced_past_survivor_still_re_picks_it() {
        let mut r = registry_with(&[
            UnitStatus::Runnable,
            UnitStatus::Runnable,
            UnitStatus::Runnable,
            UnitStatus::Runnable,
            UnitStatus::Runnable,
        ]);
        let mut s = RoundRobinScheduler::new();
        assert_eq!(s.select_next(&r), Some(UnitId::new(0)));
        assert_eq!(s.select_next(&r), Some(UnitId::new(1)));
        assert_eq!(s.select_next(&r), Some(UnitId::new(2)));
        assert_eq!(s.select_next(&r), Some(UnitId::new(3)));
        assert_eq!(s.last_scheduled(), Some(UnitId::new(3)));
        for &i in &[0u64, 1, 2, 4] {
            r.set_status_override(UnitId::new(i), UnitStatus::Blocked);
        }
        for _ in 0..5 {
            assert_eq!(s.select_next(&r), Some(UnitId::new(3)));
        }
    }

    /// Hand-traced selection sequence pinned against `[Blocked,
    /// Runnable, Faulted, Runnable, Finished]`. Cross-instance
    /// determinism tests can't catch a registry-side iter-order
    /// regression; this can.
    #[test]
    fn round_robin_select_next_matches_hand_expected_sequence() {
        let r = registry_with(&[
            UnitStatus::Blocked,
            UnitStatus::Runnable,
            UnitStatus::Faulted,
            UnitStatus::Runnable,
            UnitStatus::Finished,
        ]);
        let mut s = RoundRobinScheduler::new();
        let observed: Vec<u64> = (0..6)
            .map(|_| s.select_next(&r).expect("runnable set non-empty").raw())
            .collect();
        let expected: Vec<u64> = vec![1, 3, 1, 3, 1, 3];
        assert_eq!(
            observed, expected,
            "scheduler output drifted from the hand-expected round-robin \
             sequence; probable cause: registry.iter() is no longer ascending, \
             two-pass wrap is broken, or a status skip misfired"
        );
    }
}
