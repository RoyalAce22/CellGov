//! Both observers refuse a region the run cannot read.

use super::{observe, observe_from_boot, observe_with_determinism_check};
use super::{BootOutcome, DeterminismError, ObserveError, RegionDescriptor, RegionExtractError};
use crate::identity::RunIdentity;
use crate::runner_cellgov::region::SpaceSnapshots;
use cellgov_core::AddressSpaceId;
use cellgov_mem::GuestMemory;
use cellgov_testkit::fixtures;
use cellgov_testkit::runner::run;

#[test]
fn observe_from_boot_refuses_a_region_the_run_cannot_read() {
    let spaces = SpaceSnapshots::from([(AddressSpaceId::BOOT, GuestMemory::new(16))]);
    let past_the_end = RegionDescriptor {
        name: "past_the_end".into(),
        space: AddressSpaceId::BOOT,
        addr: 16,
        size: 1,
    };
    let err = observe_from_boot(
        &spaces,
        BootOutcome::ProcessExit,
        1,
        std::slice::from_ref(&past_the_end),
        &[],
        RunIdentity::default(),
    )
    .expect_err("byte 16 is the first byte past a 16-byte space");
    assert!(
        matches!(
            &err,
            RegionExtractError::Unreadable {
                name,
                space: 0,
                addr: 16,
                size: 1,
                ..
            } if name == "past_the_end"
        ),
        "{err:?}"
    );
}

#[test]
fn a_region_the_run_cannot_read_fails_observation_rather_than_reading_as_zeros() {
    let result = run(fixtures::dma_block_unblock_scenario());
    let past_the_end = RegionDescriptor {
        name: "past_the_end".into(),
        space: AddressSpaceId::BOOT,
        addr: 0x10000,
        size: 16,
    };
    let err = observe(&result, std::slice::from_ref(&past_the_end))
        .expect_err("the dma scenario maps 256 bytes; 0x10000 is past the end");
    assert!(
        matches!(&err, ObserveError::Region(RegionExtractError::Unreadable { name, .. }) if name == "past_the_end"),
        "{err:?}"
    );
}

#[test]
fn the_determinism_check_refuses_a_region_in_a_space_the_run_never_created() {
    let ghost = RegionDescriptor {
        name: "ghost".into(),
        space: AddressSpaceId::new(7),
        addr: 0,
        size: 16,
    };
    let err = observe_with_determinism_check(
        fixtures::dma_block_unblock_scenario,
        std::slice::from_ref(&ghost),
    )
    .expect_err("a synthetic scenario creates only the boot space");
    assert!(
        matches!(
            &err,
            DeterminismError::Observe(ObserveError::Region(RegionExtractError::SpaceMissing {
                name,
                space: 7,
                ..
            })) if name == "ghost"
        ),
        "{err:?}"
    );
}
