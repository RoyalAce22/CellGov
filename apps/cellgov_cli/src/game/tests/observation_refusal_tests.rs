//! Refusal of a manifest region the run cannot read.

use super::{save_boot_observation, ObservationInputs, ObservationSaveError};

#[test]
fn a_region_past_the_end_of_its_space_is_refused_and_nothing_is_written() {
    let dir = cellgov_testkit::scratch::scratch_labeled("past_the_end");
    let out = dir.join("observation.json");
    let spaces: cellgov_compare::SpaceSnapshots = [(
        cellgov_compare::AddressSpaceId::BOOT,
        cellgov_mem::GuestMemory::new(0x1000),
    )]
    .into_iter()
    .collect();
    let regions = vec![cellgov_compare::RegionDescriptor {
        name: "past_the_end".into(),
        space: cellgov_compare::AddressSpaceId::BOOT,
        addr: 0x1000,
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
    })
    .expect_err("0x1000 is the first byte past the 0x1000-byte space");
    assert!(
        matches!(
            &err,
            ObservationSaveError::Region(cellgov_compare::RegionExtractError::Unreadable {
                name,
                ..
            }) if name == "past_the_end"
        ),
        "{err:?}"
    );
    assert!(
        !out.exists(),
        "a refused region must not leave a zero-filled observation behind"
    );
}
