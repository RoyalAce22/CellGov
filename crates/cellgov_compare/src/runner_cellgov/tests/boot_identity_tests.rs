//! The identity the boot adapter carries into an observation.

use super::*;
use crate::runner_cellgov::region::SpaceSnapshots;
use cellgov_core::AddressSpaceId;
use cellgov_mem::GuestMemory;

fn boot_only(size: usize) -> SpaceSnapshots {
    SpaceSnapshots::from([(AddressSpaceId::BOOT, GuestMemory::new(size))])
}

#[test]
fn observe_from_boot_carries_the_identity_through() {
    let mem = boot_only(16);
    let id = crate::test_support::identity("4.91", "NPAA00001", "base");
    let obs = observe_from_boot(&mem, BootOutcome::ProcessExit, 1, &[], &[], id.clone())
        .expect("no regions requested");
    assert_eq!(obs.identity, id);
}
