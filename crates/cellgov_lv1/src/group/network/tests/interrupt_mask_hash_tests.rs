use super::*;
use cellgov_mem::lanes::{contribution, LaneIndex};

fn masked(mask: u64) -> NetworkState {
    let mut state = NetworkState::default();
    state.apply(NetworkChange::InterruptMaskSet(mask));
    state
}

/// The list holds every nonzero lane of a state with a mask set.
#[test]
fn interrupt_mask_sync_term_lane_vector_golden() {
    let lane = |field, value| contribution(LaneIndex::new(234, 0, field, 0), value);
    let sum = [lane(0, 1), lane(1, 0x1002_0000_0000_6000)]
        .into_iter()
        .fold(0u128, u128::wrapping_add);
    assert_eq!(masked(0x1002_0000_0000_6000).sync_term(), sum);
}

#[test]
fn each_mask_moves_the_term_and_adds_to_the_other_terms() {
    let one = masked(0x1002_0000_0000_6000).sync_term();
    assert_ne!(one, 0);
    assert_ne!(masked(0xC000_0000).sync_term(), one);
    assert_ne!(masked(0).sync_term(), 0, "a mask of 0 is present");
    let mut all = masked(0x1002_0000_0000_6000);
    all.apply(NetworkChange::StatusIndicatorSet(0x0012_3458));
    all.apply(NetworkChange::InterruptPortConnected(19));
    let mut others = NetworkState::default();
    others.apply(NetworkChange::StatusIndicatorSet(0x0012_3458));
    others.apply(NetworkChange::InterruptPortConnected(19));
    assert_eq!(all.sync_term(), one.wrapping_add(others.sync_term()));
}

#[test]
fn a_later_mask_replaces_the_earlier_one() {
    let mut state = masked(0x1002_0000_0000_6000);
    state.apply(NetworkChange::InterruptMaskSet(0xC000_0000));
    assert_eq!(state.interrupt_mask(), Some(0xC000_0000));
    assert_eq!(state.sync_term(), masked(0xC000_0000).sync_term());
}

#[test]
fn the_interrupt_mask_surface_is_on_its_own_source() {
    let surfaces: Vec<_> = GROUP
        .hashed_surfaces
        .iter()
        .filter(|surface| surface.source == Some(INTERRUPT_MASK_SOURCE))
        .collect();
    assert_eq!(surfaces.len(), 1);
    assert!(GROUP.lanes.iter().any(|lanes| lanes.contains(&234)));
}
