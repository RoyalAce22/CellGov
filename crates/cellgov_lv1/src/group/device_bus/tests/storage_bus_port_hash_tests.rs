use super::*;
use cellgov_mem::lanes::{contribution, LaneIndex};

fn connected(id: u64) -> DeviceBusState {
    let mut state = DeviceBusState::default();
    state.apply(DeviceBusChange::StorageBusPortConnected(id));
    state
}

/// The list holds every nonzero lane of a state with a connected port.
#[test]
fn storage_bus_port_sync_term_lane_vector_golden() {
    let lane = |field, value| contribution(LaneIndex::new(180, 0, field, 0), value);
    let sum = [lane(0, 1), lane(1, 20)]
        .into_iter()
        .fold(0u128, u128::wrapping_add);
    assert_eq!(connected(20).sync_term(), sum);
    assert_eq!(connected(0).sync_term(), lane(0, 1));
}

#[test]
fn the_port_id_moves_the_term_and_adds_to_the_storage_bus_open_term() {
    assert_ne!(connected(20).sync_term(), connected(21).sync_term());
    let open = StorageBusOpen { r5: 0 };
    let mut both = connected(20);
    both.apply(DeviceBusChange::StorageBusOpened(open));
    let open_term = lanes::value_term(STORAGE_BUS_OPEN_SOURCE, 0, &open);
    assert_eq!(
        both.sync_term(),
        connected(20).sync_term().wrapping_add(open_term)
    );
    assert_eq!(both.storage_bus_port(), Some(20));
}

#[test]
fn the_storage_bus_port_surface_is_on_its_own_source() {
    let surfaces: Vec<_> = GROUP
        .hashed_surfaces
        .iter()
        .filter(|surface| surface.source == Some(STORAGE_BUS_PORT_SOURCE))
        .collect();
    assert_eq!(surfaces.len(), 1);
    assert_eq!(STORAGE_BUS_PORT_SOURCE, 180);
}
