//! Regression tests for exploration classifications.
//!
//! These tests lock down the classification of known workloads. If a
//! code change alters the classification of a workload that was
//! previously schedule-stable or schedule-sensitive, these tests fail
//! and the change must be investigated.

use cellgov_core::Runtime;
use cellgov_exec::fake_isa::{FakeIsaUnit, FakeOp};
use cellgov_explore::{explore, ExplorationConfig, OutcomeClass};
use cellgov_mem::GuestMemory;
use cellgov_time::Budget;

fn default_config() -> ExplorationConfig {
    ExplorationConfig::default()
}

// -- Schedule-stable regressions --

/// Two units writing to disjoint addresses: always schedule-stable.
#[test]
fn regression_disjoint_writes_stable() {
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
        &default_config(),
    );
    let r = result.expect("branching points exist");
    assert_eq!(
        r.outcome,
        OutcomeClass::ScheduleStable,
        "disjoint writes must remain schedule-stable"
    );
}

/// Three units writing to disjoint addresses: always schedule-stable.
#[test]
fn regression_three_disjoint_writers_stable() {
    let result = explore(
        || {
            let mem = GuestMemory::new(64);
            let mut rt = Runtime::new(mem, Budget::new(100), 100);
            for (i, val) in [0xAA, 0xBB, 0xCC].iter().enumerate() {
                let addr = (i * 8) as u64;
                let v = *val;
                rt.registry_mut().register_with(move |id| {
                    FakeIsaUnit::new(
                        id,
                        vec![
                            FakeOp::LoadImm(v),
                            FakeOp::SharedStore { addr, len: 4 },
                            FakeOp::End,
                        ],
                    )
                });
            }
            rt
        },
        &default_config(),
    );
    let r = result.expect("branching points exist");
    assert_eq!(
        r.outcome,
        OutcomeClass::ScheduleStable,
        "three disjoint writers must remain schedule-stable"
    );
}

// -- Schedule-sensitive regressions --

/// Two units writing to the SAME address: always schedule-sensitive.
#[test]
fn regression_overlapping_writes_sensitive() {
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
        &default_config(),
    );
    let r = result.expect("branching points exist");
    assert_eq!(
        r.outcome,
        OutcomeClass::ScheduleSensitive,
        "overlapping writes must remain schedule-sensitive"
    );
}

/// Three units writing to the same address: always schedule-sensitive.
#[test]
fn regression_three_overlapping_writers_sensitive() {
    let result = explore(
        || {
            let mem = GuestMemory::new(64);
            let mut rt = Runtime::new(mem, Budget::new(100), 100);
            for val in [0xAA, 0xBB, 0xCC] {
                rt.registry_mut().register_with(move |id| {
                    FakeIsaUnit::new(
                        id,
                        vec![
                            FakeOp::LoadImm(val),
                            FakeOp::SharedStore { addr: 0, len: 4 },
                            FakeOp::End,
                        ],
                    )
                });
            }
            rt
        },
        &default_config(),
    );
    let r = result.expect("branching points exist");
    assert_eq!(
        r.outcome,
        OutcomeClass::ScheduleSensitive,
        "three overlapping writers must remain schedule-sensitive"
    );
}

// -- No branching regressions --

/// Single unit: no branching points, explore returns None.
#[test]
fn regression_single_unit_no_branching() {
    let result = explore(
        || {
            let mem = GuestMemory::new(64);
            let mut rt = Runtime::new(mem, Budget::new(100), 100);
            rt.registry_mut()
                .register_with(|id| FakeIsaUnit::new(id, vec![FakeOp::End]));
            rt
        },
        &default_config(),
    );
    assert!(
        result.is_none(),
        "single unit must produce no branching points"
    );
}

// -- Pruning regressions --

/// Disjoint writers should be pruned by dependency analysis.
#[test]
fn regression_disjoint_writers_pruned() {
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
        &default_config(),
    );
    let r = result.expect("branching points exist");
    assert!(
        r.schedules_pruned > 0,
        "disjoint writers must be pruned by dependency analysis"
    );
    assert!(
        r.schedules.is_empty(),
        "all alternates should be pruned for disjoint writers"
    );
}

/// Overlapping writers should NOT be pruned.
#[test]
fn regression_overlapping_writers_not_pruned() {
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
        &default_config(),
    );
    let r = result.expect("branching points exist");
    assert_eq!(
        r.schedules_pruned, 0,
        "overlapping writers must not be pruned"
    );
    assert!(
        !r.schedules.is_empty(),
        "overlapping writers should have explored alternates"
    );
}
