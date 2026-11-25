use super::*;
use cellgov_mem::lanes::{contribution, LaneIndex};

fn requested(tag: u8, completion: Option<u64>) -> StorageState {
    let mut state = StorageState::default();
    state.apply(StorageChange::NotifyRequested(NotifyRequest {
        tag,
        completion,
    }));
    state
}

/// The list holds every nonzero lane of a state with one request,
/// completed or waiting. An empty state has no lane.
#[test]
fn notify_request_sync_term_lane_vector_golden() {
    let lane = |field, value| contribution(LaneIndex::new(208, 3, field, 0), value);
    let sum = [lane(0, 1), lane(1, 3), lane(2, 1), lane(3, 5)]
        .into_iter()
        .fold(0u128, u128::wrapping_add);
    assert_eq!(requested(3, Some(5)).sync_term(), sum);
    let waiting = [lane(0, 1), lane(1, 3)]
        .into_iter()
        .fold(0u128, u128::wrapping_add);
    assert_eq!(requested(3, None).sync_term(), waiting);
    assert_eq!(StorageState::default().sync_term(), 0);
}

/// The list holds every nonzero lane of a state with an event mask.
#[test]
fn notify_event_mask_sync_term_lane_vector_golden() {
    let lane = |field, value| contribution(LaneIndex::new(209, 0, field, 0), value);
    let mut state = StorageState::default();
    state.apply(StorageChange::NotifyEventMaskSet(2));
    assert_eq!(state.sync_term(), lane(0, 1).wrapping_add(lane(1, 2)));
    assert_eq!(state.notify_event_mask(), Some(2));
}

#[test]
fn the_tag_and_the_completion_move_the_request_term() {
    assert_ne!(
        requested(1, Some(0)).sync_term(),
        requested(2, Some(0)).sync_term()
    );
    assert_ne!(
        requested(1, Some(0)).sync_term(),
        requested(1, Some(5)).sync_term()
    );
    assert_ne!(
        requested(1, Some(0)).sync_term(),
        requested(1, None).sync_term()
    );
}

#[test]
fn each_request_holds_its_tag_bit() {
    let mut state = requested(1, Some(0));
    state.apply(StorageChange::NotifyRequested(NotifyRequest {
        tag: 15,
        completion: None,
    }));
    assert_eq!(state.held_tags(), (1 << 1) | (1 << 15));
    assert_eq!(state.notify_requests().count(), 2);
}

#[test]
fn each_storage_lane_source_has_one_hashed_surface() {
    for source in [NOTIFY_REQUEST_SOURCE, NOTIFY_EVENT_MASK_SOURCE] {
        let surfaces = GROUP
            .hashed_surfaces
            .iter()
            .filter(|surface| surface.source == Some(source))
            .count();
        assert_eq!(surfaces, 1, "source {source}");
    }
    assert_eq!(
        (NOTIFY_REQUEST_SOURCE, NOTIFY_EVENT_MASK_SOURCE),
        (208, 209)
    );
}
