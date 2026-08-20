//! Pairwise schedule exploration over fake-ISA runtimes: disjoint writes stable, overlapping writes sensitive.

use super::*;
use cellgov_core::AddressSpaceId;
use cellgov_exec::fake_isa::{FakeIsaUnit, FakeOp};
use cellgov_mem::{GuestMemory, PageSize};
use cellgov_time::Budget;

#[test]
fn cross_space_pair_is_schedule_stable() {
    // Same numeric address in two address spaces: a legal
    // cross-process schedule whose interleavings all commit the same
    // per-space state.
    let result = explore_pair(|| {
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
        rt.assign_unit_space(cellgov_event::UnitId::new(1), AddressSpaceId::new(1))
            .unwrap();
        rt
    });

    let r = result.expect("should have a branching point");
    assert!(
        r.is_schedule_stable(),
        "same numeric address in disjoint spaces must not alias across schedules"
    );
}

#[test]
fn child_space_only_divergence_is_not_schedule_stable() {
    // Both racers live in the child space; the boot space's bytes are
    // identical across schedules, so the pair verdict stands or falls
    // on the multi-space committed hash.
    let result = explore_pair(|| {
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
        rt.assign_unit_space(cellgov_event::UnitId::new(0), AddressSpaceId::new(1))
            .unwrap();
        rt.assign_unit_space(cellgov_event::UnitId::new(1), AddressSpaceId::new(1))
            .unwrap();
        rt
    });

    let r = result.expect("should have a branching point");
    assert!(
        !r.is_schedule_stable(),
        "a last-writer race confined to a child space must be witnessed"
    );
}

#[test]
fn disjoint_writes_are_schedule_stable() {
    let result = explore_pair(|| {
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
    });

    let r = result.expect("should have a branching point");
    assert!(
        r.is_schedule_stable(),
        "disjoint writes should produce identical memory regardless of order"
    );
}

#[test]
fn overlapping_writes_are_schedule_sensitive() {
    let result = explore_pair(|| {
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
    });

    let r = result.expect("should have a branching point");
    assert!(
        !r.is_schedule_stable(),
        "overlapping writes should produce different memory depending on order"
    );
}

#[test]
fn no_branching_returns_none() {
    let result = explore_pair(|| {
        let mem = GuestMemory::new(64);
        let mut rt = Runtime::new(mem, Budget::new(100), 100);
        rt.registry_mut()
            .register_with(|id| FakeIsaUnit::new(id, vec![FakeOp::End]));
        rt
    });
    assert!(result.is_none());
}
