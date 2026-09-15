//! Refusal of a manifest region whose bytes are provisional zeros.

use super::{save_boot_observation, ObservationInputs, ObservationSaveError};
use cellgov_mem::{PageSize, Region, RegionAccess};
use cellgov_ps3_abi::hw::address_space::PS3_RSX_BASE;

#[test]
fn a_region_over_the_reserved_rsx_window_is_refused_and_nothing_is_written() {
    let dir = cellgov_testkit::scratch::scratch_labeled("provisional_rsx");
    let out = dir.join("observation.json");
    let mem = cellgov_mem::GuestMemory::from_regions(vec![
        Region::new(0, 0x1000, "main", PageSize::Page64K),
        Region::with_access(
            PS3_RSX_BASE,
            0x1000,
            "rsx",
            PageSize::Page4K,
            RegionAccess::ReservedZeroReadable,
        ),
    ])
    .unwrap();
    let spaces: cellgov_compare::SpaceSnapshots = [(cellgov_compare::AddressSpaceId::BOOT, mem)]
        .into_iter()
        .collect();
    let regions = vec![cellgov_compare::RegionDescriptor {
        name: "rsx_window".into(),
        space: cellgov_compare::AddressSpaceId::BOOT,
        addr: PS3_RSX_BASE + 0x40,
        size: 16,
    }];

    let err = save_boot_observation(ObservationInputs {
        path: out.to_str().unwrap(),
        elf_data: &[],
        final_spaces: &spaces,
        outcome: cellgov_compare::BootOutcome::ProcessExit,
        steps: 0,
        manifest_regions: Some(&regions),
        tty_log: &[],
        identity: &cellgov_compare::RunIdentity::default(),
        sink: &crate::NullSink,
    })
    .expect_err("the RSX window reads as provisional zeros");
    assert!(
        matches!(
            &err,
            ObservationSaveError::Region(cellgov_compare::RegionExtractError::Provisional {
                name,
                region: "rsx",
                ..
            }) if name == "rsx_window"
        ),
        "{err:?}"
    );
    assert!(
        !out.exists(),
        "a refused region must not leave an observation of modeled zeros behind"
    );
}
