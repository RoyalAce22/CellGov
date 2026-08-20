//! Full schedule exploration over fake-ISA runtimes: outcome classification and reproducibility under contention.

use super::*;
use crate::classify::OutcomeClass;
use cellgov_core::AddressSpaceId;
use cellgov_event::UnitId;
use cellgov_exec::fake_isa::{FakeIsaUnit, FakeOp};
use cellgov_mem::{GuestMemory, PageSize};
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
fn explore_cross_space_same_address_is_stable() {
    // The overlapping-writes fixture that classifies sensitive in one
    // space: with the second unit in its own address space, the equal
    // numeric addresses name different memory and every interleaving
    // commits the same final state.
    let result = explore(
        || {
            let mem = GuestMemory::new(64);
            let mut rt = Runtime::new(mem, Budget::new(100), 100);
            rt.create_address_space(AddressSpaceId::new(1)).unwrap();
            rt.space_memory_mut(AddressSpaceId::new(1))
                .unwrap()
                .install_region(0, 64, "child", PageSize::Page64K)
                .unwrap();
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
            rt.assign_unit_space(UnitId::new(1), AddressSpaceId::new(1))
                .unwrap();
            rt
        },
        &ExplorationConfig::default(),
    );

    let r = result.expect("should have branching points");
    assert_eq!(r.outcome, OutcomeClass::ScheduleStable);
}

#[test]
fn explore_child_space_only_divergence_is_sensitive() {
    // Both racers live in the child space and boot memory never
    // changes, so only the multi-space committed hash can witness the
    // last-writer split; a boot-only hash would classify this stable.
    let result = explore(
        || {
            let mem = GuestMemory::new(64);
            let mut rt = Runtime::new(mem, Budget::new(100), 100);
            rt.create_address_space(AddressSpaceId::new(1)).unwrap();
            rt.space_memory_mut(AddressSpaceId::new(1))
                .unwrap()
                .install_region(0, 64, "child", PageSize::Page64K)
                .unwrap();
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
            rt.assign_unit_space(UnitId::new(0), AddressSpaceId::new(1))
                .unwrap();
            rt.assign_unit_space(UnitId::new(1), AddressSpaceId::new(1))
                .unwrap();
            rt
        },
        &ExplorationConfig::default(),
    );

    let r = result.expect("should have branching points");
    assert_eq!(r.outcome, OutcomeClass::ScheduleSensitive);
}

#[test]
fn explore_shared_view_cross_space_is_sensitive() {
    // Different numeric addresses, same shared bytes: the alias
    // expansion must keep the pair from being pruned as independent,
    // and the multi-space hash must witness the last-writer split.
    let result = explore(
        || {
            let mem = GuestMemory::new(64);
            let mut rt = Runtime::new(mem, Budget::new(100), 100);
            rt.create_address_space(AddressSpaceId::new(1)).unwrap();
            rt.register_shared_mapping(
                11,
                0x40,
                &[
                    (AddressSpaceId::BOOT, 0x2000),
                    (AddressSpaceId::new(1), 0x3000),
                ],
            )
            .unwrap();
            rt.registry_mut().register_with(|id| {
                FakeIsaUnit::new(
                    id,
                    vec![
                        FakeOp::LoadImm(0xAA),
                        FakeOp::SharedStore {
                            addr: 0x2000,
                            len: 4,
                        },
                        FakeOp::End,
                    ],
                )
            });
            rt.registry_mut().register_with(|id| {
                FakeIsaUnit::new(
                    id,
                    vec![
                        FakeOp::LoadImm(0xBB),
                        FakeOp::SharedStore {
                            addr: 0x3000,
                            len: 4,
                        },
                        FakeOp::End,
                    ],
                )
            });
            rt.assign_unit_space(UnitId::new(1), AddressSpaceId::new(1))
                .unwrap();
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
