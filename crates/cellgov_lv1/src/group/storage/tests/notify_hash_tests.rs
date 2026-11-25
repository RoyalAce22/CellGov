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

/// The list holds every nonzero lane of a state with one completed
/// request.
#[test]
fn notify_request_sync_term_lane_vector_golden() {
    let lane = |field, value| contribution(LaneIndex::new(208, 1, field, 0), value);
    let sum = [lane(0, 1), lane(1, 1), lane(2, 1)]
        .into_iter()
        .fold(0u128, u128::wrapping_add);
    assert_eq!(requested(1, Some(0)).sync_term(), sum);
    let waiting = [lane(0, 1), lane(1, 1)]
        .into_iter()
        .fold(0u128, u128::wrapping_add);
    assert_eq!(requested(1, None).sync_term(), waiting);
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
    assert_eq!(StorageState::default().sync_term(), 0);
}

#[test]
fn the_storage_surfaces_are_pending_on_their_own_sources() {
    for source in [NOTIFY_REQUEST_SOURCE, NOTIFY_EVENT_MASK_SOURCE] {
        let surfaces: Vec<_> = GROUP
            .hashed_surfaces
            .iter()
            .filter(|surface| surface.source == Some(source))
            .collect();
        assert_eq!(surfaces.len(), 1, "source {source}");
        assert_eq!(surfaces[0].stamp, Stamp::Pending, "source {source}");
    }
    assert_eq!(
        (NOTIFY_REQUEST_SOURCE, NOTIFY_EVENT_MASK_SOURCE),
        (208, 209)
    );
}
