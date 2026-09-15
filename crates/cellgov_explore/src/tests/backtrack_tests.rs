//! Backtrack-set search over fake-ISA runtimes: outcome, bounds, and what each replay records.

use super::*;
use crate::classify::OutcomeClass;
use crate::util::StopReason;
use cellgov_core::Runtime;
use cellgov_event::UnitId;
use cellgov_exec::fake_isa::{FakeIsaUnit, FakeOp};
use cellgov_mem::GuestMemory;
use cellgov_time::Budget;
use std::cell::Cell;
use std::collections::BTreeSet;

/// Two writers over the same four bytes, so the order decides the
/// final contents.
fn overlapping_writers() -> Runtime {
    let mut rt = Runtime::new(GuestMemory::new(64), Budget::new(1), 100);
    for value in [0xAAu32, 0xBB] {
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
    rt
}

fn disjoint_writers() -> Runtime {
    let mut rt = Runtime::new(GuestMemory::new(64), Budget::new(1), 100);
    for (value, addr) in [(0xAAu32, 0u64), (0xBB, 8)] {
        rt.register_unit_with(|id| {
            FakeIsaUnit::new(
                id,
                vec![
                    FakeOp::LoadImm(value),
                    FakeOp::SharedStore { addr, len: 4 },
                    FakeOp::End,
                ],
            )
        });
    }
    rt
}

#[test]
fn overlapping_writers_are_schedule_sensitive() {
    let result = explore_backtrack(overlapping_writers, &ExplorationConfig::default());
    assert_eq!(result.baseline_stop, StopReason::Stalled);
    assert_eq!(result.outcome, OutcomeClass::ScheduleSensitive);
}

#[test]
fn disjoint_writers_are_schedule_stable() {
    let result = explore_backtrack(disjoint_writers, &ExplorationConfig::default());
    assert_eq!(result.baseline_stop, StopReason::Stalled);
    assert_eq!(result.outcome, OutcomeClass::ScheduleStable);
    assert!(!result.bounds_hit);
}

#[test]
fn a_workload_with_no_race_explores_one_execution() {
    let result = explore_backtrack(disjoint_writers, &ExplorationConfig::default());
    assert!(
        result.total_branching_points > 0,
        "the workload offers choices, so no replay means no race rather \
         than no choice",
    );
    assert!(result.schedules.is_empty());
}

/// The baseline retires six steps and its only race holds the two
/// stores apart, at steps 2 and 3. So both backtrack points sit at step
/// 2, and the two replays force one unit each.
#[test]
fn every_replay_records_the_step_it_forced() {
    let result = explore_backtrack(overlapping_writers, &ExplorationConfig::default());
    assert_eq!(result.baseline_steps, 6);
    assert_eq!(result.schedules.len(), 2);
    let forced: BTreeSet<(usize, UnitId)> = result
        .schedules
        .iter()
        .map(|record| (record.branch_step, record.alternate_choice))
        .collect();
    assert_eq!(
        forced,
        BTreeSet::from([(2, UnitId::new(0)), (2, UnitId::new(1))]),
    );
}

#[test]
fn a_step_bound_on_the_baseline_withdraws_the_verdict() {
    let config = ExplorationConfig {
        max_schedules: 256,
        max_steps_per_run: 2,
    };
    let result = explore_backtrack(overlapping_writers, &config);
    assert_eq!(result.baseline_stop, StopReason::StepBound);
    assert_eq!(result.baseline_steps, 2);
    assert_eq!(result.outcome, OutcomeClass::Inconclusive);
    assert!(result.bounds_hit);
}

/// Two steps end before either store, so the baseline holds no race.
#[test]
fn a_truncated_baseline_names_no_backtrack_point() {
    let config = ExplorationConfig {
        max_schedules: 256,
        max_steps_per_run: 2,
    };
    let result = explore_backtrack(overlapping_writers, &config);
    assert!(result.baseline_stop.is_truncated());
    assert!(result.schedules.is_empty());
    assert_eq!(result.schedules_truncated, 0);
}

#[test]
fn a_break_only_a_replay_found_reaches_the_result() {
    // A logged break changes no guest state, so every run steps the
    // same workload in the same order.
    let runs = Cell::new(0usize);
    let result = explore_backtrack(
        || {
            let mut rt = overlapping_writers();
            if runs.get() > 0 {
                rt.lv2_host_mut()
                    .log_invariant_break("test.site", format_args!("details here"));
            }
            runs.set(runs.get() + 1);
            rt
        },
        &ExplorationConfig::default(),
    );
    assert!(
        !result.schedules.is_empty(),
        "setup: contending writers race, so the search replays at least once",
    );
    assert_eq!(
        result.first_invariant_break.as_deref(),
        Some("lv2 host invariant break at test.site: details here (the first of 1)"),
    );
}
