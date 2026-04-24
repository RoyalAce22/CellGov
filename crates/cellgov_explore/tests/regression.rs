//! Regression tests pinning the classification of known workloads.

use cellgov_core::Runtime;
use cellgov_exec::fake_isa::{FakeIsaUnit, FakeOp};
use cellgov_explore::{explore, ExplorationConfig, OutcomeClass};
use cellgov_mem::GuestMemory;
use cellgov_time::Budget;

fn default_config() -> ExplorationConfig {
    ExplorationConfig::default()
}

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

/// Flat region at base 0 plus an RSX region at 0xC000_0000, with
/// the put-pointer writeback mirror enabled.
fn build_rsx_runtime() -> Runtime {
    use cellgov_mem::{PageSize, Region};
    let regions = vec![
        Region::new(0, 0x1000, "flat", PageSize::Page4K),
        Region::new(0xC000_0000, 0x1000, "rsx", PageSize::Page64K),
    ];
    let mem = GuestMemory::from_regions(regions).expect("non-overlapping regions");
    let mut rt = Runtime::new(mem, Budget::new(100), 100);
    rt.set_rsx_mirror_writes(true);
    rt
}

#[test]
fn regression_rsx_put_write_stable_vs_disjoint_write() {
    let result = explore(
        || {
            let mut rt = build_rsx_runtime();
            rt.registry_mut().register_with(|id| {
                FakeIsaUnit::new(
                    id,
                    vec![
                        FakeOp::LoadImm(0xAB),
                        FakeOp::SharedStore {
                            addr: 0xC000_0040,
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
                        FakeOp::LoadImm(0xCD),
                        FakeOp::SharedStore { addr: 0x10, len: 4 },
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
        "put-pointer write + disjoint write must remain schedule-stable"
    );
}

#[test]
fn regression_rsx_two_writers_to_same_control_slot_sensitive() {
    let result = explore(
        || {
            let mut rt = build_rsx_runtime();
            rt.registry_mut().register_with(|id| {
                FakeIsaUnit::new(
                    id,
                    vec![
                        FakeOp::LoadImm(0xAB),
                        FakeOp::SharedStore {
                            addr: 0xC000_0040,
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
                        FakeOp::LoadImm(0xCD),
                        FakeOp::SharedStore {
                            addr: 0xC000_0040,
                            len: 4,
                        },
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
        "two writers to the same RSX control slot must be schedule-sensitive"
    );
}
