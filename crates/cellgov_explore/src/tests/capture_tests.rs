use super::*;
use crate::config::ExplorationConfig;
use cellgov_exec::fake_isa::{FakeIsaUnit, FakeOp};
use cellgov_mem::GuestMemory;
use cellgov_time::Budget;

#[test]
fn an_unmapped_region_spec_is_captured_unresolved_not_zero_filled() {
    // The 64-byte fixture memory ends well before 0x1_0000, so the
    // spec's range cannot be read from any run's committed memory.
    let specs = vec![MemoryRegionSpec {
        name: "outside".into(),
        space: AddressSpaceId::BOOT,
        addr: 0x1_0000,
        size: 4,
    }];
    let result = explore_with_regions(
        || {
            let mem = GuestMemory::new(64);
            let mut rt = Runtime::new(mem, Budget::new(100), 100);
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
            rt.register_unit_with(|id| {
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
    assert!(!r.baseline.regions[0].resolved);
    assert!(r.baseline.regions[0].data.is_empty());
    assert!(
        !r.alternates.is_empty(),
        "overlapping writers must produce at least one explored alternate"
    );
    for alt in &r.alternates {
        assert!(!alt.regions[0].resolved);
        assert!(alt.regions[0].data.is_empty());
    }
}

#[test]
fn a_mapped_region_spec_is_captured_resolved() {
    let specs = vec![MemoryRegionSpec {
        name: "inside".into(),
        space: AddressSpaceId::BOOT,
        addr: 0,
        size: 4,
    }];
    let result = explore_with_regions(
        || {
            let mem = GuestMemory::new(64);
            let mut rt = Runtime::new(mem, Budget::new(100), 100);
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
            rt.register_unit_with(|id| {
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
    assert!(r.baseline.regions[0].resolved);
    assert_eq!(r.baseline.regions[0].data.len(), 4);
}
