use super::*;
use cellgov_mem::lanes::{contribution, LaneIndex};

fn set(address: u64) -> NetworkState {
    let mut state = NetworkState::default();
    state.apply(NetworkChange::StatusIndicatorSet(address));
    state
}

#[test]
fn a_state_with_no_indicator_adds_no_term() {
    assert_eq!(NetworkState::default().sync_term(), 0);
}

/// The list holds every nonzero lane of a state with the indicator set.
#[test]
fn status_indicator_sync_term_lane_vector_golden() {
    let lane = |field, value| contribution(LaneIndex::new(232, 0, field, 0), value);
    let sum = [lane(0, 1), lane(1, 0x0012_3458)]
        .into_iter()
        .fold(0u128, u128::wrapping_add);
    assert_eq!(set(0x0012_3458).sync_term(), sum);
}

#[test]
fn each_indicator_address_moves_the_term() {
    let one = set(0x0012_3458).sync_term();
    assert_ne!(one, 0);
    assert_ne!(set(0x0012_3460).sync_term(), one);
    assert_ne!(
        set(0).sync_term(),
        0,
        "an indicator at address 0 is present"
    );
}

#[test]
fn the_status_indicator_surface_is_on_its_own_source() {
    let surfaces: Vec<_> = GROUP
        .hashed_surfaces
        .iter()
        .filter(|surface| surface.source == Some(STATUS_INDICATOR_SOURCE))
        .collect();
    assert_eq!(surfaces.len(), 1);
    assert!(GROUP.lanes.iter().any(|lanes| lanes.contains(&232)));
}
