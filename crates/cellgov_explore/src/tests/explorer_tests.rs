//! Full schedule exploration over fake-ISA runtimes: outcome classification and reproducibility under contention.

use super::*;
use crate::classify::OutcomeClass;
use cellgov_exec::fake_isa::{FakeIsaUnit, FakeOp};
use cellgov_mem::GuestMemory;
use cellgov_time::Budget;

#[test]
fn explore_disjoint_writes_is_stable() {
    let result = explore(
        || {
            let mem = GuestMemory::new(64);
            let mut rt = Runtime::new(mem, Budget::new(100), 100);
            rt.registry_mut().register_with(|id| {
                FakeIsaUnit::new(
                    id,
                    vec![
                        FakeOp::LoadImm(0xAA),
                        FakeOp::SharedStore { addr: 0, len: 4 },
                        FakeOp::End,
                    ],
                )
            });
            rt.registry_mut().register_with(|id| {
                FakeIsaUnit::new(
                    id,
                    vec![
                        FakeOp::LoadImm(0xBB),
                        FakeOp::SharedStore { addr: 8, len: 4 },
                        FakeOp::End,
                    ],
                )
            });
            rt
        },
        &ExplorationConfig::default(),
    );

    let r = result.expect("should have branching points");
    assert_eq!(r.outcome, OutcomeClass::ScheduleStable);
    assert!(!r.bounds_hit);
}

#[test]
fn explore_two_unit_atomic_contention_is_reproducible() {
    let make = || {
        let mem = GuestMemory::new(256);
        let mut rt = Runtime::new(mem, Budget::new(100), 100);
        rt.registry_mut().register_with(|id| {
            FakeIsaUnit::new(
                id,
                vec![
                    FakeOp::LoadImm(0xAA),
                    FakeOp::ReservationAcquire { line_addr: 0x80 },
                    FakeOp::ConditionalStore { addr: 0x80, len: 4 },
                    FakeOp::End,
                ],
            )
        });
        rt.registry_mut().register_with(|id| {
            FakeIsaUnit::new(
                id,
                vec![
                    FakeOp::LoadImm(0xBB),
                    FakeOp::ReservationAcquire { line_addr: 0x80 },
                    FakeOp::ConditionalStore { addr: 0x80, len: 4 },
                    FakeOp::End,
                ],
            )
        });
        rt
    };

    let r1 = explore(make, &ExplorationConfig::default())
        .expect("contention workload must have branching points");
    let r2 = explore(make, &ExplorationConfig::default())
        .expect("contention workload must have branching points");

    assert_eq!(
        r1.outcome, r2.outcome,
        "exploration classification must be stable across runs",
    );
    assert!(matches!(
        r1.outcome,
        OutcomeClass::ScheduleStable | OutcomeClass::ScheduleSensitive
    ));
}

#[test]
fn explore_two_unit_atomic_same_value_is_stable() {
    let result = explore(
        || {
            let mem = GuestMemory::new(256);
            let mut rt = Runtime::new(mem, Budget::new(100), 100);
            for _ in 0..2 {
                rt.registry_mut().register_with(|id| {
                    FakeIsaUnit::new(
                        id,
                        vec![
                            FakeOp::LoadImm(0x42),
                            FakeOp::ReservationAcquire { line_addr: 0x80 },
                            FakeOp::ConditionalStore { addr: 0x80, len: 4 },
                            FakeOp::End,
                        ],
                    )
                });
            }
            rt
        },
        &ExplorationConfig::default(),
    )
    .expect("contention workload must have branching points");

    assert_eq!(
        result.outcome,
        OutcomeClass::ScheduleStable,
        "matching conditional-store values across both units must collapse to one class",
    );
}

#[test]
fn explore_overlapping_writes_is_sensitive() {
    let result = explore(
        || {
            let mem = GuestMemory::new(64);
            let mut rt = Runtime::new(mem, Budget::new(100), 100);
            rt.registry_mut().register_with(|id| {
                FakeIsaUnit::new(
                    id,
                    vec![
                        FakeOp::LoadImm(0xAA),
                        FakeOp::SharedStore { addr: 0, len: 4 },
                        FakeOp::End,
                    ],
                )
            });
            rt.registry_mut().register_with(|id| {
                FakeIsaUnit::new(
                    id,
                    vec![
                        FakeOp::LoadImm(0xBB),
                        FakeOp::SharedStore { addr: 0, len: 4 },
                        FakeOp::End,
                    ],
                )
            });
            rt
        },
        &ExplorationConfig::default(),
    );

    let r = result.expect("should have branching points");
    assert_eq!(r.outcome, OutcomeClass::ScheduleSensitive);
}

#[test]
fn single_unit_returns_none() {
    let result = explore(
        || {
            let mem = GuestMemory::new(64);
            let mut rt = Runtime::new(mem, Budget::new(100), 100);
            rt.registry_mut()
                .register_with(|id| FakeIsaUnit::new(id, vec![FakeOp::End]));
            rt
        },
        &ExplorationConfig::default(),
    );
    assert!(result.is_none());
}

#[test]
fn three_unit_disjoint_is_stable() {
    let result = explore(
        || {
            let mem = GuestMemory::new(64);
            let mut rt = Runtime::new(mem, Budget::new(100), 100);
            rt.registry_mut().register_with(|id| {
                FakeIsaUnit::new(
                    id,
                    vec![
                        FakeOp::LoadImm(0xAA),
                        FakeOp::SharedStore { addr: 0, len: 4 },
                        FakeOp::End,
                    ],
                )
            });
            rt.registry_mut().register_with(|id| {
                FakeIsaUnit::new(
                    id,
                    vec![
                        FakeOp::LoadImm(0xBB),
                        FakeOp::SharedStore { addr: 8, len: 4 },
                        FakeOp::End,
                    ],
                )
            });
            rt.registry_mut().register_with(|id| {
                FakeIsaUnit::new(
                    id,
                    vec![
                        FakeOp::LoadImm(0xCC),
                        FakeOp::SharedStore { addr: 16, len: 4 },
                        FakeOp::End,
                    ],
                )
            });
            rt
        },
        &ExplorationConfig::default(),
    );

    let r = result.expect("should have branching points");
    assert_eq!(r.outcome, OutcomeClass::ScheduleStable);
    assert!(r.total_branching_points >= 2);
    assert!(r.schedules.is_empty());
    assert!(r.schedules_pruned >= 3);
}

#[test]
fn three_unit_overlapping_is_sensitive() {
    let result = explore(
        || {
            let mem = GuestMemory::new(64);
            let mut rt = Runtime::new(mem, Budget::new(100), 100);
            rt.registry_mut().register_with(|id| {
                FakeIsaUnit::new(
                    id,
                    vec![
                        FakeOp::LoadImm(0xAA),
                        FakeOp::SharedStore { addr: 0, len: 4 },
                        FakeOp::End,
                    ],
                )
            });
            rt.registry_mut().register_with(|id| {
                FakeIsaUnit::new(
                    id,
                    vec![
                        FakeOp::LoadImm(0xBB),
                        FakeOp::SharedStore { addr: 0, len: 4 },
                        FakeOp::End,
                    ],
                )
            });
            rt.registry_mut().register_with(|id| {
                FakeIsaUnit::new(
                    id,
                    vec![
                        FakeOp::LoadImm(0xCC),
                        FakeOp::SharedStore { addr: 0, len: 4 },
                        FakeOp::End,
                    ],
                )
            });
            rt
        },
        &ExplorationConfig::default(),
    );

    let r = result.expect("should have branching points");
    assert_eq!(r.outcome, OutcomeClass::ScheduleSensitive);
}

#[test]
fn max_schedules_bound_produces_inconclusive() {
    let config = ExplorationConfig {
        max_schedules: 1,
        max_steps_per_run: 10_000,
    };
    let result = explore(
        || {
            let mem = GuestMemory::new(64);
            let mut rt = Runtime::new(mem, Budget::new(100), 100);
            rt.registry_mut().register_with(|id| {
                FakeIsaUnit::new(
                    id,
                    vec![
                        FakeOp::LoadImm(0xAA),
                        FakeOp::SharedStore { addr: 0, len: 4 },
                        FakeOp::End,
                    ],
                )
            });
            rt.registry_mut().register_with(|id| {
                FakeIsaUnit::new(
                    id,
                    vec![
                        FakeOp::LoadImm(0xBB),
                        FakeOp::SharedStore { addr: 0, len: 4 },
                        FakeOp::End,
                    ],
                )
            });
            rt.registry_mut().register_with(|id| {
                FakeIsaUnit::new(
                    id,
                    vec![
                        FakeOp::LoadImm(0xCC),
                        FakeOp::SharedStore { addr: 0, len: 4 },
                        FakeOp::End,
                    ],
                )
            });
            rt
        },
        &config,
    );

    let r = result.expect("should have branching points");
    assert_eq!(r.schedules.len(), 1, "should stop after 1 schedule");
    assert!(r.bounds_hit);
    // The one explored alternate swaps LoadImm order but not the
    // last writer, so no divergence is visible; bounds-hit without
    // divergence classifies as Inconclusive.
    assert_eq!(r.outcome, OutcomeClass::Inconclusive);
}

#[test]
fn result_fields_are_populated() {
    let result = explore(
        || {
            let mem = GuestMemory::new(64);
            let mut rt = Runtime::new(mem, Budget::new(100), 100);
            rt.registry_mut().register_with(|id| {
                FakeIsaUnit::new(
                    id,
                    vec![
                        FakeOp::LoadImm(0xAA),
                        FakeOp::SharedStore { addr: 0, len: 4 },
                        FakeOp::End,
                    ],
                )
            });
            rt.registry_mut().register_with(|id| {
                FakeIsaUnit::new(
                    id,
                    vec![
                        FakeOp::LoadImm(0xBB),
                        FakeOp::SharedStore { addr: 0, len: 4 },
                        FakeOp::End,
                    ],
                )
            });
            rt
        },
        &ExplorationConfig::default(),
    );

    let r = result.expect("should have branching points");
    assert!(r.baseline_hash != 0, "baseline hash should be non-zero");
    assert!(!r.schedules.is_empty());
    assert!(r.total_branching_points >= 1);
    assert_eq!(r.outcome, OutcomeClass::ScheduleSensitive);
}

#[test]
fn disjoint_pruning_skips_all_alternates() {
    let result = explore(
        || {
            let mem = GuestMemory::new(64);
            let mut rt = Runtime::new(mem, Budget::new(100), 100);
            rt.registry_mut().register_with(|id| {
                FakeIsaUnit::new(
                    id,
                    vec![
                        FakeOp::LoadImm(0xAA),
                        FakeOp::SharedStore { addr: 0, len: 4 },
                        FakeOp::End,
                    ],
                )
            });
            rt.registry_mut().register_with(|id| {
                FakeIsaUnit::new(
                    id,
                    vec![
                        FakeOp::LoadImm(0xBB),
                        FakeOp::SharedStore { addr: 8, len: 4 },
                        FakeOp::End,
                    ],
                )
            });
            rt
        },
        &ExplorationConfig::default(),
    );

    let r = result.expect("should have branching points");
    assert_eq!(r.outcome, OutcomeClass::ScheduleStable);
    assert!(r.schedules.is_empty(), "all alternates should be pruned");
    assert!(
        r.schedules_pruned > 0,
        "pruning should have skipped at least one alternate"
    );
}
