use super::*;
use cellgov_mem::lanes::{contribution, LaneIndex};

fn held(r5: u64) -> DeviceBusState {
    let mut state = DeviceBusState::default();
    state.apply(DeviceBusChange::StorageBusOpened(StorageBusOpen { r5 }));
    state
}

/// The list holds every nonzero lane of a state with the storage bus open.
#[test]
fn storage_bus_open_sync_term_lane_vector_golden() {
    let lane = |field, value| contribution(LaneIndex::new(179, 0, field, 0), value);
    let sum = [lane(0, 1), lane(1, 0x8000_0000_0000_0001)]
        .into_iter()
        .fold(0u128, u128::wrapping_add);
    assert_eq!(held(0x8000_0000_0000_0001).sync_term(), sum);
    assert_eq!(held(0).sync_term(), lane(0, 1));
}

#[test]
fn the_r5_moves_the_term_and_the_open_adds_to_the_open_device_term() {
    assert_ne!(held(1).sync_term(), held(0).sync_term());
    assert_ne!(held(0).sync_term(), 0);
    let device = OpenDevice { bus: 1, device: 0 };
    let mut both = held(0);
    both.apply(DeviceBusChange::Opened(device));
    let open_term = lanes::value_term(OPEN_DEVICE_SOURCE, 0, &device);
    assert_eq!(
        both.sync_term(),
        held(0).sync_term().wrapping_add(open_term)
    );
}

#[test]
fn the_storage_bus_open_surface_is_on_its_own_source() {
    let surfaces: Vec<_> = GROUP
        .hashed_surfaces
        .iter()
        .filter(|surface| surface.source == Some(STORAGE_BUS_OPEN_SOURCE))
        .collect();
    assert_eq!(surfaces.len(), 1);
    assert_eq!(STORAGE_BUS_OPEN_SOURCE, 179);
}
