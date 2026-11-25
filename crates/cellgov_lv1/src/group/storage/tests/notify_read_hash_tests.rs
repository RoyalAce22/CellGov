use super::*;
use cellgov_mem::lanes::{contribution, LaneIndex};

const READ: WaitingRead = WaitingRead {
    buffer: 0x3B_3000,
    tag: 1,
};

fn after(changes: &[StorageChange]) -> StorageState {
    let mut state = StorageState::default();
    for change in changes {
        state.apply(change.clone());
    }
    state
}

/// The list holds every nonzero lane of a state with two reported
/// devices, and of a state with one waiting read.
#[test]
fn notify_read_sync_term_lane_vector_golden() {
    let lane = |source, field, value| contribution(LaneIndex::new(source, 0, field, 0), value);
    let reported = after(&[
        StorageChange::NotifyDeviceReported,
        StorageChange::NotifyDeviceReported,
    ]);
    assert_eq!(
        reported.sync_term(),
        lane(211, 0, 1).wrapping_add(lane(211, 1, 2))
    );
    let waiting = after(&[StorageChange::NotifyReadWaiting(READ)]);
    assert_eq!(
        waiting.sync_term(),
        lane(212, 0, 1)
            .wrapping_add(lane(212, 1, 0x3B_3000))
            .wrapping_add(lane(212, 2, 1))
    );
}

#[test]
fn each_report_and_each_waiting_read_moves_the_term() {
    let none = StorageState::default().sync_term();
    let one = after(&[StorageChange::NotifyDeviceReported]).sync_term();
    let two = after(&[
        StorageChange::NotifyDeviceReported,
        StorageChange::NotifyDeviceReported,
    ])
    .sync_term();
    let waiting = after(&[StorageChange::NotifyReadWaiting(READ)]).sync_term();
    let other_tag = after(&[StorageChange::NotifyReadWaiting(WaitingRead {
        tag: 2,
        ..READ
    })])
    .sync_term();
    let other_buffer = after(&[StorageChange::NotifyReadWaiting(WaitingRead {
        buffer: 0x3B_4000,
        ..READ
    })])
    .sync_term();
    assert_eq!(none, 0);
    assert_ne!(one, none);
    assert_ne!(two, one);
    assert_ne!(waiting, none);
    assert_ne!(other_tag, waiting);
    assert_ne!(other_buffer, waiting);
}

/// The list holds every nonzero lane of a state that holds all of these:
/// - a request;
/// - an event mask;
/// - a registered device;
/// - two reports;
/// - a waiting read.
#[test]
fn every_storage_surface_adds_its_lanes_to_one_term() {
    let lane = |source, object, field, value| {
        contribution(LaneIndex::new(source, object, field, 0), value)
    };
    let state = after(&[
        StorageChange::NotifyRequested(NotifyRequest {
            tag: 1,
            completion: None,
        }),
        StorageChange::NotifyEventMaskSet(2),
        StorageChange::DeviceRegistered(3),
        StorageChange::NotifyDeviceReported,
        StorageChange::NotifyDeviceReported,
        StorageChange::NotifyReadWaiting(READ),
    ]);
    let sum = [
        lane(208, 1, 0, 1),
        lane(208, 1, 1, 1),
        lane(209, 0, 0, 1),
        lane(209, 0, 1, 2),
        lane(210, 3, 0, 1),
        lane(210, 3, 1, 3),
        lane(211, 0, 0, 1),
        lane(211, 0, 1, 2),
        lane(212, 0, 0, 1),
        lane(212, 0, 1, 0x3B_3000),
        lane(212, 0, 2, 1),
    ]
    .into_iter()
    .fold(0u128, u128::wrapping_add);
    assert_eq!(state.sync_term(), sum);
    assert_eq!(state.registered_devices().collect::<Vec<_>>(), [3]);
    assert_eq!(state.reported_devices(), 2);
    assert_eq!(state.waiting_read(), Some(READ));
}

#[test]
fn a_report_ends_the_waiting_read_and_raises_the_count() {
    let state = after(&[
        StorageChange::NotifyReadWaiting(READ),
        StorageChange::NotifyDeviceReported,
    ]);
    assert_eq!(state.waiting_read(), None);
    assert_eq!(state.reported_devices(), 1);
    assert_eq!(
        state.sync_term(),
        after(&[StorageChange::NotifyDeviceReported]).sync_term()
    );
}
