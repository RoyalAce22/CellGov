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

/// The lane of field `field` of object `object` of the fixed source 82,
/// with the value `value`.
fn lane(object: u64, field: u8, value: u64) -> u128 {
    contribution(LaneIndex::new(82, object, field, 0), value)
}

/// The list holds every nonzero lane of a state with one port, of id 3.
#[test]
fn event_receive_port_sync_term_lane_vector_golden() {
    let sum = [lane(0, 0, 1), lane(0, 1, 3)]
        .into_iter()
        .fold(0u128, u128::wrapping_add);
    assert_eq!(ports(&[3]).sync_term(), sum);
}

/// The list holds every nonzero lane of a state with the ports 4 and 3,
/// constructed in that order: id 3 is object 0 and id 4 is object 1.
#[test]
fn two_ports_take_objects_0_and_1_in_id_order_golden() {
    let sum = [lane(0, 0, 1), lane(0, 1, 3), lane(1, 0, 1), lane(1, 1, 4)]
        .into_iter()
        .fold(0u128, u128::wrapping_add);
    assert_eq!(ports(&[4, 3]).sync_term(), sum);
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
