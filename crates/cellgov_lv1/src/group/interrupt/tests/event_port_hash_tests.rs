use super::*;
use cellgov_mem::lanes::{contribution, LaneIndex};

fn ports(ids: &[u64]) -> EventReceivePortState {
    let mut state = EventReceivePortState::default();
    for &id in ids {
        state.construct(EventReceivePort::new(id));
    }
    state
}

#[test]
fn a_port_state_with_no_port_adds_no_term() {
    assert_eq!(EventReceivePortState::default().sync_term(), 0);
}

/// The list holds every nonzero lane of a state with one port, of id 3.
#[test]
fn event_receive_port_sync_term_lane_vector_golden() {
    let lane = |object, field, value| {
        contribution(
            LaneIndex::new(EVENT_RECEIVE_PORT_SOURCE, object, field, 0),
            value,
        )
    };
    let sum = [lane(0, 0, 1), lane(0, 1, 3)]
        .into_iter()
        .fold(0u128, u128::wrapping_add);
    assert_eq!(ports(&[3]).sync_term(), sum);
}

#[test]
fn each_port_and_each_id_moves_the_term() {
    let one = ports(&[3]);
    assert_ne!(one.sync_term(), 0);
    assert_ne!(ports(&[3, 4]).sync_term(), one.sync_term());
    assert_ne!(ports(&[4]).sync_term(), one.sync_term());
}

#[test]
fn the_term_does_not_depend_on_the_construction_order() {
    assert_eq!(ports(&[4, 3]), ports(&[3, 4]));
    assert_eq!(ports(&[4, 3]).sync_term(), ports(&[3, 4]).sync_term());
}
