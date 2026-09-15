//! Whole-struct equality over an [`ExplorationResult`].
//!
//! An outcome alone hides a field that moved: two runs can classify a
//! workload the same way while disagreeing on which schedules they
//! reached, what each one hashed, or how many classes they covered. A
//! search is a pure function of its workload, so two runs give one
//! whole struct, and the cases below name every field of it.

#![allow(
    clippy::unwrap_used,
    reason = "integration test: a panic on unexpected failure is the right behavior"
)]

use cellgov_core::Runtime;
use cellgov_exec::fake_isa::{FakeIsaUnit, FakeOp};
use cellgov_explore::{
    explore_backtrack, explore_window, ExplorationConfig, ExplorationResult, OutcomeClass,
    StopReason,
};
use cellgov_mem::GuestMemory;
use cellgov_time::Budget;

/// Three units over one address and one over its own, so a result has
/// several schedules, more than one hash, and a class count to compare.
fn mixed() -> Runtime {
    let mut rt = Runtime::new(GuestMemory::new(64), Budget::new(1), 200);
    for value in [0xAAu32, 0xBB, 0xCC] {
        rt.register_unit_with(|id| {
            FakeIsaUnit::new(
                id,
                vec![
                    FakeOp::LoadImm(value),
                    FakeOp::SharedStore { addr: 0, len: 4 },
                    FakeOp::End,
                ],
            )
        });
    }
    rt.register_unit_with(|id| {
        FakeIsaUnit::new(
            id,
            vec![
                FakeOp::LoadImm(0xDD),
                FakeOp::SharedStore { addr: 32, len: 4 },
                FakeOp::End,
            ],
        )
    });
    rt
}

#[test]
fn the_optimal_search_gives_one_answer() {
    let config = ExplorationConfig::default();
    assert_eq!(
        explore_window(mixed, &config),
        explore_window(mixed, &config),
    );
}

#[test]
fn the_backtrack_search_gives_one_answer() {
    let config = ExplorationConfig::default();
    assert_eq!(
        explore_backtrack(mixed, &config),
        explore_backtrack(mixed, &config),
    );
}

/// The scaffold can reach one class more than once, so its execution
/// count is no class count and it claims none; the optimal search's
/// count over the same workload is the control.
#[test]
fn the_backtrack_search_claims_no_class_count() {
    let config = ExplorationConfig::default();
    let scaffold = explore_backtrack(mixed, &config);
    assert!(!scaffold.bounds_hit);
    assert_eq!(scaffold.classes_explored, None);
    assert!(explore_window(mixed, &config).classes_explored.is_some());
}

/// The destructure is the guard: a field added to the struct stops this
/// compiling until someone decides what it should say here.
#[test]
fn every_field_of_a_finished_result_is_named() {
    let result = explore_window(mixed, &ExplorationConfig::default());
    let ExplorationResult {
        baseline_hash,
        baseline_steps,
        baseline_stop,
        schedules,
        outcome,
        total_branching_points,
        classes_explored,
        reversals_dropped,
        bounds_hit,
        schedules_pruned,
        schedules_truncated,
        schedules_refused,
        first_invariant_break,
    } = result;

    assert_ne!(baseline_hash, 0);
    assert_eq!(baseline_steps, 12, "four units of three steps each");
    assert_eq!(baseline_stop, StopReason::Stalled);
    assert_eq!(outcome, OutcomeClass::ScheduleSensitive);
    assert!(total_branching_points > 0);
    assert_eq!(
        classes_explored,
        Some(schedules.len() + 1),
        "the search covered one execution per class",
    );
    assert_eq!(
        reversals_dropped, 0,
        "a run that claims a count owed no reversal it could not deliver",
    );
    assert_eq!(
        classes_explored,
        Some(6),
        "three conflicting stores order six ways; the fourth unit is disjoint",
    );
    assert!(!bounds_hit);
    assert_eq!(schedules_pruned, 0, "no execution stopped on a sleep set");
    assert_eq!(schedules_truncated, 0);
    assert_eq!(schedules_refused, 0);
    assert_eq!(first_invariant_break, None);
    assert!(
        schedules
            .iter()
            .any(|record| record.memory_hash != baseline_hash),
        "the workload is schedule-sensitive, so some schedule disagrees",
    );
}

/// A driver that explores a window of a longer run hands over a runtime
/// it already advanced to the window's start; it has nothing to build a
/// second one from.
#[test]
fn the_search_builds_one_runtime() {
    let mut once = Some(mixed());
    let result = explore_window(
        || {
            once.take()
                .expect("the search builds one runtime and restores it")
        },
        &ExplorationConfig::default(),
    );
    assert!(
        !result.schedules.is_empty(),
        "the workload races, so the search ran more than the baseline",
    );
}
