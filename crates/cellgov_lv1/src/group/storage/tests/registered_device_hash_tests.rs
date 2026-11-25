use super::*;
use cellgov_mem::lanes::{contribution, LaneIndex};

fn registered(devices: &[u64]) -> StorageState {
    let mut state = StorageState::default();
    for &device in devices {
        state.apply(StorageChange::DeviceRegistered(device));
    }
    state
}

/// The list holds every nonzero lane of a state with one registered
/// device.
#[test]
fn registered_device_sync_term_lane_vector_golden() {
    let lane = |object, field, value| contribution(LaneIndex::new(210, object, field, 0), value);
    assert_eq!(
        registered(&[3]).sync_term(),
        lane(3, 0, 1).wrapping_add(lane(3, 1, 3))
    );
    assert_eq!(registered(&[0]).sync_term(), lane(0, 0, 1));
}

#[test]
fn each_registered_device_moves_the_term() {
    let none = StorageState::default().sync_term();
    let one = registered(&[1]).sync_term();
    let two = registered(&[1, 4]).sync_term();
    let other = registered(&[2]).sync_term();
    assert_eq!(none, 0);
    assert_ne!(one, none);
    assert_ne!(two, one);
    assert_ne!(other, one);
}

#[test]
fn a_second_registration_of_a_device_keeps_the_state() {
    let once = registered(&[2]);
    let twice = registered(&[2, 2]);
    assert_eq!(once, twice);
    assert_eq!(twice.registered_devices().collect::<Vec<_>>(), [2]);
}
