//! Named memory-region capture during exploration and per-region divergence across schedules.

use super::*;
use crate::classify::OutcomeClass;
use cellgov_exec::fake_isa::{FakeIsaUnit, FakeOp};
use cellgov_mem::GuestMemory;
use cellgov_time::Budget;

#[test]
fn explore_with_regions_captures_disjoint_writes() {
    let specs = vec![
        MemoryRegionSpec {
            name: "region_a".into(),
            addr: 0,
            size: 4,
        },
        MemoryRegionSpec {
            name: "region_b".into(),
            addr: 8,
            size: 4,
        },
    ];
    let result = explore_with_regions(
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
        &specs,
    );

    let r = result.expect("should have branching points");
    assert_eq!(r.baseline.regions.len(), 2);
    assert_eq!(r.baseline.regions[0].name, "region_a");
    assert_eq!(r.baseline.regions[0].data, vec![0xAA; 4]);
    assert_eq!(r.baseline.regions[1].name, "region_b");
    assert_eq!(r.baseline.regions[1].data, vec![0xBB; 4]);
}

#[test]
fn overlapping_writes_regions_differ_across_schedules() {
    let specs = vec![MemoryRegionSpec {
        name: "shared".into(),
        addr: 0,
        size: 4,
    }];
    let result = explore_with_regions(
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
        &specs,
    );

    let r = result.expect("should have branching points");
    assert_eq!(r.exploration.outcome, OutcomeClass::ScheduleSensitive);
    let baseline_data = &r.baseline.regions[0].data;
    let any_different = r
        .alternates
        .iter()
        .any(|s| s.regions[0].data != *baseline_data);
    assert!(any_different, "at least one alternate should differ");
}
