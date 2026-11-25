use super::*;
use cellgov_mem::lanes::{contribution, LaneIndex};

/// The list holds every nonzero lane of a state that holds an open with
/// r3 = 5.
#[test]
fn gpu_open_sync_term_lane_vector_golden() {
    let lane = |field, value| contribution(LaneIndex::new(216, 0, field, 0), value);
    let sum = lane(0, 1).wrapping_add(lane(1, 5));
    let mut state = GpuState::default();
    assert_eq!(state.sync_term(), 0);
    state.apply(GpuChange::Opened(GpuOpen { r3: 5 }));
    assert_eq!(state.sync_term(), sum);
}

#[test]
fn an_open_with_r3_0_adds_a_term() {
    let mut state = GpuState::default();
    state.apply(GpuChange::Opened(GpuOpen { r3: 0 }));
    let presence = contribution(LaneIndex::new(216, 0, 0, 0), 1);
    assert_eq!(state.sync_term(), presence);
    assert_ne!(state.sync_term(), 0);
}

#[test]
fn the_gpu_open_surface_is_on_its_own_source() {
    assert_eq!(GROUP.hashed_surfaces.len(), 1);
    let surface = &GROUP.hashed_surfaces[0];
    assert_eq!(surface.source, Some(GPU_OPEN_SOURCE));
    assert_eq!(surface.stamp, Stamp::Pending);
    assert_eq!(GPU_OPEN_SOURCE, 216);
    assert!(GROUP
        .lanes
        .iter()
        .any(|range| range.contains(&GPU_OPEN_SOURCE)));
}
