use super::*;
use cellgov_mem::lanes::{contribution, LaneIndex};

fn connected(id: u64) -> NetworkState {
    let mut state = NetworkState::default();
    state.apply(NetworkChange::InterruptPortConnected(id));
    state
}

/// The list holds every nonzero lane of a state with a connected port.
#[test]
fn interrupt_port_sync_term_lane_vector_golden() {
    let lane = |field, value| contribution(LaneIndex::new(233, 0, field, 0), value);
    let sum = [lane(0, 1), lane(1, 19)]
        .into_iter()
        .fold(0u128, u128::wrapping_add);
    assert_eq!(connected(19).sync_term(), sum);
}

#[test]
fn each_port_id_moves_the_term_and_adds_to_the_indicator_term() {
    let one = connected(19).sync_term();
    assert_ne!(one, 0);
    assert_ne!(connected(20).sync_term(), one);
    assert_ne!(connected(0).sync_term(), 0, "port id 0 is present");
    let mut both = connected(19);
    both.apply(NetworkChange::StatusIndicatorSet(0x0012_3458));
    let mut indicator = NetworkState::default();
    indicator.apply(NetworkChange::StatusIndicatorSet(0x0012_3458));
    assert_eq!(both.sync_term(), one.wrapping_add(indicator.sync_term()));
}

#[test]
fn the_interrupt_port_surface_is_on_its_own_source() {
    let surfaces: Vec<_> = GROUP
        .hashed_surfaces
        .iter()
        .filter(|surface| surface.source == Some(INTERRUPT_PORT_SOURCE))
        .collect();
    assert_eq!(surfaces.len(), 1);
    assert!(GROUP.lanes.iter().any(|lanes| lanes.contains(&233)));
}
