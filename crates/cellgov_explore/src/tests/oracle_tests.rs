//! Named memory-region capture during exploration and per-region divergence across schedules.

use super::*;
use crate::classify::OutcomeClass;
use cellgov_event::UnitId;
use cellgov_exec::fake_isa::{FakeIsaUnit, FakeOp};
use cellgov_mem::{GuestMemory, PageSize, Region};
use cellgov_time::Budget;

fn spec(name: &str, space: AddressSpaceId, addr: u64, size: u64) -> MemoryRegionSpec {
    MemoryRegionSpec {
        name: name.into(),
        space,
        addr,
        size,
    }
}

fn store_unit(rt: &mut Runtime, imm: u32, addr: u64) {
    rt.register_unit_with(|id| {
        FakeIsaUnit::new(
            id,
            vec![
                FakeOp::LoadImm(imm),
                FakeOp::SharedStore { addr, len: 4 },
                FakeOp::End,
            ],
        )
    });
}

#[test]
fn explore_with_regions_captures_disjoint_writes() {
    let specs = vec![
        spec("region_a", AddressSpaceId::BOOT, 0, 4),
        spec("region_b", AddressSpaceId::BOOT, 8, 4),
    ];
    let result = explore_with_regions(
        || {
            let mem = GuestMemory::new(64);
            let mut rt = Runtime::new(mem, Budget::new(100), 100);
            store_unit(&mut rt, 0xAA, 0);
            store_unit(&mut rt, 0xBB, 8);
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
    let specs = vec![spec("shared", AddressSpaceId::BOOT, 0, 4)];
    let result = explore_with_regions(
        || {
            let mem = GuestMemory::new(64);
            let mut rt = Runtime::new(mem, Budget::new(100), 100);
            store_unit(&mut rt, 0xAA, 0);
            store_unit(&mut rt, 0xBB, 0);
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

/// Two units in a child space race on one word; boot memory never
/// changes.
fn child_space_racers() -> Runtime {
    let mem = GuestMemory::new(64);
    let mut rt = Runtime::new(mem, Budget::new(100), 100);
    let child =
        GuestMemory::from_regions(vec![Region::new(0, 64, "child", PageSize::Page64K)]).unwrap();
    rt.create_address_space_with(AddressSpaceId::new(1), child)
        .unwrap();
    store_unit(&mut rt, 0xAA, 0);
    store_unit(&mut rt, 0xBB, 0);
    rt.assign_unit_space(UnitId::new(0), AddressSpaceId::new(1))
        .unwrap();
    rt.assign_unit_space(UnitId::new(1), AddressSpaceId::new(1))
        .unwrap();
    rt
}

#[test]
fn a_child_space_spec_captures_the_child_side_of_the_race() {
    let specs = vec![
        spec("boot_view", AddressSpaceId::BOOT, 0, 4),
        spec("child_view", AddressSpaceId::new(1), 0, 4),
    ];
    let r = explore_with_regions(child_space_racers, &ExplorationConfig::default(), &specs)
        .expect("two racers must produce a branching point");

    assert_eq!(r.exploration.outcome, OutcomeClass::ScheduleSensitive);
    for snap in std::iter::once(&r.baseline).chain(&r.alternates) {
        assert!(snap.regions.iter().all(|c| c.resolved));
        assert_eq!(snap.regions[0].data, vec![0; 4], "boot memory is untouched");
    }
    let child_values: Vec<&Vec<u8>> = std::iter::once(&r.baseline)
        .chain(&r.alternates)
        .map(|s| &s.regions[1].data)
        .collect();
    assert!(
        child_values.iter().any(|d| **d != child_values[0][..]),
        "the child-space capture must witness the last-writer split: {child_values:?}"
    );
    assert!(child_values
        .iter()
        .all(|d| **d == [0xAA; 4] || **d == [0xBB; 4]));
}

#[test]
fn a_spec_naming_a_space_the_run_never_created_is_unresolved() {
    let specs = vec![spec("nowhere", AddressSpaceId::new(7), 0, 4)];
    let r = explore_with_regions(child_space_racers, &ExplorationConfig::default(), &specs)
        .expect("two racers must produce a branching point");
    for snap in std::iter::once(&r.baseline).chain(&r.alternates) {
        assert!(!snap.regions[0].resolved);
        assert!(snap.regions[0].data.is_empty());
    }
}
