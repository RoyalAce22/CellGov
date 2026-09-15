//! Optimal DPOR over fake-ISA runtimes.

use super::*;
use crate::classify::OutcomeClass;
use cellgov_exec::fake_isa::{FakeIsaUnit, FakeOp};
use cellgov_mem::GuestMemory;
use cellgov_time::Budget;

/// `count` units that each store a distinct byte over the same four
/// bytes.
fn writers(count: u32) -> Runtime {
    let mut rt = Runtime::new(GuestMemory::new(64), Budget::new(1), 200);
    for index in 0..count {
        rt.register_unit_with(|id| {
            FakeIsaUnit::new(
                id,
                vec![
                    FakeOp::LoadImm(0xA0 + index),
                    FakeOp::SharedStore { addr: 0, len: 4 },
                    FakeOp::End,
                ],
            )
        });
    }
    rt
}

fn disjoint_writers(count: u32) -> Runtime {
    let mut rt = Runtime::new(GuestMemory::new(64), Budget::new(1), 200);
    for index in 0..count {
        rt.register_unit_with(|id| {
            FakeIsaUnit::new(
                id,
                vec![
                    FakeOp::LoadImm(0xA0 + index),
                    FakeOp::SharedStore {
                        addr: u64::from(index) * 8,
                        len: 4,
                    },
                    FakeOp::End,
                ],
            )
        });
    }
    rt
}

#[test]
fn two_writers_over_one_address_are_schedule_sensitive() {
    let result = explore_optimal(|| writers(2), &ExplorationConfig::default());
    assert_eq!(result.baseline_stop, StopReason::Stalled);
    assert_eq!(result.outcome, OutcomeClass::ScheduleSensitive);
}

#[test]
fn disjoint_writers_are_schedule_stable() {
    let result = explore_optimal(|| disjoint_writers(2), &ExplorationConfig::default());
    assert_eq!(result.outcome, OutcomeClass::ScheduleStable);
    assert!(!result.bounds_hit);
}

/// Two conflicting stores have `2! = 2` orders. Every other step is
/// local, so the class count is 2.
#[test]
fn two_writers_cost_one_execution_per_class() {
    let result = explore_optimal(|| writers(2), &ExplorationConfig::default());
    assert_eq!(result.classes_explored, Some(2));
}

/// Three conflicting stores have `3! = 6` orders.
#[test]
fn three_writers_cost_one_execution_per_class() {
    let result = explore_optimal(|| writers(3), &ExplorationConfig::default());
    assert_eq!(result.classes_explored, Some(6));
}

/// Four conflicting stores have `4! = 24` orders.
#[test]
fn four_writers_cost_one_execution_per_class() {
    let result = explore_optimal(|| writers(4), &ExplorationConfig::default());
    assert_eq!(result.classes_explored, Some(24));
}

/// Disjoint stores never conflict, so every schedule is one class.
#[test]
fn disjoint_writers_cost_one_execution() {
    let result = explore_optimal(|| disjoint_writers(3), &ExplorationConfig::default());
    assert_eq!(result.classes_explored, Some(1));
}

/// A bound leaves classes uncovered, so the search reports no count and
/// cannot answer for the workload.
#[test]
fn a_step_bound_reports_no_class_count() {
    let config = ExplorationConfig {
        max_schedules: 256,
        max_steps_per_run: 2,
    };
    let result = explore_optimal(|| writers(2), &config);
    assert_eq!(result.baseline_stop, StopReason::StepBound);
    assert_eq!(result.classes_explored, None);
    assert_eq!(result.outcome, OutcomeClass::Inconclusive);
    assert!(result.bounds_hit);
}

/// A class bound withdraws the count, but a divergence the search
/// already reached still stands: a schedule that diverges is a fact
/// about the workload, whatever the search did not get to.
#[test]
fn a_class_bound_reports_no_class_count() {
    let config = ExplorationConfig {
        max_schedules: 2,
        max_steps_per_run: 10_000,
    };
    let result = explore_optimal(|| writers(4), &config);
    assert_eq!(result.classes_explored, None);
    assert!(result.bounds_hit);
    assert_eq!(result.outcome, OutcomeClass::ScheduleSensitive);
}

/// `count` units that store one value over one address: the stores
/// conflict, so the classes are their orders, and every order commits
/// the same memory.
fn writers_of_one_value(count: u32) -> Runtime {
    let mut rt = Runtime::new(GuestMemory::new(64), Budget::new(1), 200);
    for _ in 0..count {
        rt.register_unit_with(|id| {
            FakeIsaUnit::new(
                id,
                vec![
                    FakeOp::LoadImm(0xAA),
                    FakeOp::SharedStore { addr: 0, len: 4 },
                    FakeOp::End,
                ],
            )
        });
    }
    rt
}

#[test]
fn conflicting_writers_of_one_value_are_schedule_stable() {
    let result = explore_optimal(|| writers_of_one_value(3), &ExplorationConfig::default());
    assert_eq!(result.classes_explored, Some(6));
    assert_eq!(result.outcome, OutcomeClass::ScheduleStable);
}

/// A class bound over a workload with no divergence to find leaves the
/// verdict inconclusive rather than stable.
#[test]
fn a_class_bound_without_a_divergence_is_inconclusive() {
    let config = ExplorationConfig {
        max_schedules: 2,
        max_steps_per_run: 10_000,
    };
    let result = explore_optimal(|| writers_of_one_value(3), &config);
    assert_eq!(result.classes_explored, None);
    assert!(result.bounds_hit);
    assert_eq!(result.outcome, OutcomeClass::Inconclusive);
}
