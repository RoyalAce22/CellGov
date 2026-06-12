//! Round-robin selection order, sticky-streak yield handling, and status-override visibility.

use super::*;
use cellgov_effects::Effect;
use cellgov_exec::{
    ExecutionContext, ExecutionStepResult, ExecutionUnit, LocalDiagnostics, YieldReason,
};
use cellgov_time::{Budget, InstructionCost};
use std::cell::Cell;

#[derive(Clone)]

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
            consumed_cost: InstructionCost::new(budget.raw()),
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
    r.set_status_override(UnitId::new(0), UnitStatus::Blocked);
    r.set_status_override(UnitId::new(1), UnitStatus::Blocked);
    assert_eq!(s.select_next(&r), None);
    assert_eq!(s.last_scheduled(), Some(UnitId::new(0)));
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
fn syscall_yield_with_no_wake_sticks_to_same_unit() {
    let r = registry_with(&[UnitStatus::Runnable, UnitStatus::Runnable]);
    let mut s = RoundRobinScheduler::new();
    assert_eq!(s.select_next(&r), Some(UnitId::new(0)));
    s.notify_yielded(UnitId::new(0), YieldReason::Syscall, false, false);
    assert_eq!(s.select_next(&r), Some(UnitId::new(0)));
}

#[test]
fn syscall_yield_that_wakes_others_rotates() {
    let r = registry_with(&[UnitStatus::Runnable, UnitStatus::Runnable]);
    let mut s = RoundRobinScheduler::new();
    assert_eq!(s.select_next(&r), Some(UnitId::new(0)));
    s.notify_yielded(UnitId::new(0), YieldReason::Syscall, true, false);
    assert_eq!(s.select_next(&r), Some(UnitId::new(1)));
}

#[test]
fn budget_exhausted_with_held_lwmutex_sticks_to_same_unit() {
    let r = registry_with(&[UnitStatus::Runnable, UnitStatus::Runnable]);
    let mut s = RoundRobinScheduler::new();
    assert_eq!(s.select_next(&r), Some(UnitId::new(0)));
    s.notify_yielded(UnitId::new(0), YieldReason::BudgetExhausted, false, true);
    assert_eq!(s.select_next(&r), Some(UnitId::new(0)));
}

#[test]
fn budget_exhausted_without_lwmutex_rotates() {
    let r = registry_with(&[UnitStatus::Runnable, UnitStatus::Runnable]);
    let mut s = RoundRobinScheduler::new();
    assert_eq!(s.select_next(&r), Some(UnitId::new(0)));
    s.notify_yielded(UnitId::new(0), YieldReason::BudgetExhausted, false, false);
    assert_eq!(s.select_next(&r), Some(UnitId::new(1)));
}

#[test]
fn sticky_streak_rotates_after_limit() {
    let r = registry_with(&[UnitStatus::Runnable, UnitStatus::Runnable]);
    let mut s = RoundRobinScheduler::new();
    assert_eq!(s.select_next(&r), Some(UnitId::new(0)));
    for _ in 0..(RoundRobinScheduler::STICKY_STREAK_LIMIT - 1) {
        s.notify_yielded(UnitId::new(0), YieldReason::Syscall, false, true);
        assert_eq!(s.select_next(&r), Some(UnitId::new(0)));
    }
    s.notify_yielded(UnitId::new(0), YieldReason::Syscall, false, true);
    assert_eq!(s.select_next(&r), Some(UnitId::new(1)));
}

#[test]
fn sticky_streak_resets_on_non_sticky_yield() {
    let r = registry_with(&[UnitStatus::Runnable, UnitStatus::Runnable]);
    let mut s = RoundRobinScheduler::new();
    assert_eq!(s.select_next(&r), Some(UnitId::new(0)));
    for _ in 0..(RoundRobinScheduler::STICKY_STREAK_LIMIT - 1) {
        s.notify_yielded(UnitId::new(0), YieldReason::Syscall, false, true);
    }
    s.notify_yielded(UnitId::new(0), YieldReason::Syscall, true, false);
    assert_eq!(s.select_next(&r), Some(UnitId::new(1)));
    s.notify_yielded(UnitId::new(1), YieldReason::Syscall, false, true);
    assert_eq!(s.select_next(&r), Some(UnitId::new(1)));
}

#[test]
fn waiting_sync_releases_critical_section_stickiness() {
    let r = registry_with(&[UnitStatus::Runnable, UnitStatus::Runnable]);
    let mut s = RoundRobinScheduler::new();
    assert_eq!(s.select_next(&r), Some(UnitId::new(0)));
    s.notify_yielded(UnitId::new(0), YieldReason::WaitingSync, false, true);
    assert_eq!(s.select_next(&r), Some(UnitId::new(1)));
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
    assert_eq!(observed, expected);
}
