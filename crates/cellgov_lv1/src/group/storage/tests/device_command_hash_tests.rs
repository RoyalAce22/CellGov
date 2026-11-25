use super::*;
use cellgov_mem::lanes::{contribution, LaneIndex};

fn started(commands: &[DeviceCommand]) -> StorageState {
    let mut state = StorageState::default();
    for &command in commands {
        state.apply(StorageChange::DeviceCommandStarted(command));
    }
    state
}

fn query(device: u64, tag: u8) -> DeviceCommand {
    DeviceCommand {
        device,
        tag,
        command: 0x1A,
        data: 0x4D_0160,
        data_bytes: 12,
    }
}

/// The list holds every nonzero lane of a state with one command.
#[test]
fn device_command_sync_term_lane_vector_golden() {
    let lane = |object, field, value| contribution(LaneIndex::new(213, object, field, 0), value);
    let expected = [
        lane(33, 0, 1),
        lane(33, 1, 2),
        lane(33, 2, 1),
        lane(33, 3, 0x1A),
        lane(33, 4, 0x4D_0160),
        lane(33, 5, 12),
    ]
    .into_iter()
    .fold(0u128, u128::wrapping_add);
    assert_eq!(started(&[query(2, 1)]).sync_term(), expected);
}

#[test]
fn each_device_command_moves_the_term_and_holds_its_tag() {
    let one = started(&[query(2, 0)]);
    let two = started(&[query(2, 0), query(2, 3)]);
    assert_eq!(StorageState::default().sync_term(), 0);
    assert_ne!(one.sync_term(), 0);
    assert_ne!(two.sync_term(), one.sync_term());
    assert_ne!(started(&[query(3, 0)]).sync_term(), one.sync_term());
    assert_eq!(two.device_held_tags(2), 0b1001);
    assert_eq!(two.device_held_tags(3), 0);
}
