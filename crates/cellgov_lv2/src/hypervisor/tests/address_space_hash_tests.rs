use super::*;
use cellgov_mem::lanes::{contribution, LaneIndex};

fn boot_space() -> AddressSpace {
    AddressSpace {
        vas_id: 2,
        htab_size_exp: 18,
        page_size_count: 2,
        page_shifts: 0x1014_0000_0000_0000,
    }
}

#[test]
fn an_address_space_state_with_no_space_adds_no_term() {
    assert_eq!(AddressSpaceState::default().sync_term(), 0);
}

/// The list holds every nonzero lane of a state with one address space
/// that is selected.
#[test]
fn address_space_sync_term_lane_vector_golden() {
    let mut spaces = AddressSpaceState::default();
    spaces.apply(AddressSpaceChange::Constructed(boot_space()));
    spaces.apply(AddressSpaceChange::Selected(2));
    let lane =
        |src, object, field, value| contribution(LaneIndex::new(src, object, field, 0), value);
    let lanes = [
        lane(source::HYPERVISOR_SELECTED_ADDRESS_SPACE, 0, 0, 1),
        lane(source::HYPERVISOR_SELECTED_ADDRESS_SPACE, 0, 1, 2),
        lane(source::HYPERVISOR_ADDRESS_SPACE, 0, 0, 1),
        lane(source::HYPERVISOR_ADDRESS_SPACE, 0, 1, 2),
        lane(source::HYPERVISOR_ADDRESS_SPACE, 0, 2, 18),
        lane(source::HYPERVISOR_ADDRESS_SPACE, 0, 3, 2),
        lane(
            source::HYPERVISOR_ADDRESS_SPACE,
            0,
            4,
            0x1014_0000_0000_0000,
        ),
    ];
    let sum = lanes.into_iter().fold(0u128, u128::wrapping_add);
    assert_eq!(spaces.sync_term(), sum);
}

#[test]
fn the_selection_moves_the_term() {
    let mut spaces = AddressSpaceState::default();
    spaces.apply(AddressSpaceChange::Constructed(boot_space()));
    let before = spaces.sync_term();
    spaces.apply(AddressSpaceChange::Selected(2));
    assert_ne!(spaces.sync_term(), before);
}
