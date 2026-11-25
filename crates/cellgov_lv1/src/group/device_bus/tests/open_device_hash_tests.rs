use super::*;
use cellgov_mem::lanes::{contribution, LaneIndex};

fn state(devices: &[OpenDevice]) -> DeviceBusState {
    let mut state = DeviceBusState::default();
    for &device in devices {
        state.apply(DeviceBusChange::Opened(device));
    }
    state
}

const BUS_1_DEVICE_0: OpenDevice = OpenDevice { bus: 1, device: 0 };

#[test]
fn a_state_with_no_open_device_adds_no_term() {
    assert_eq!(DeviceBusState::default().sync_term(), 0);
}

/// The list holds every nonzero lane of a state with bus 1 device 0 open.
#[test]
fn open_device_sync_term_lane_vector_golden() {
    let lane = |field, value| contribution(LaneIndex::new(176, 0, field, 0), value);
    let sum = [lane(0, 1), lane(1, 1)]
        .into_iter()
        .fold(0u128, u128::wrapping_add);
    assert_eq!(state(&[BUS_1_DEVICE_0]).sync_term(), sum);
}

/// Opens of bus 2 device 5 and then bus 1 device 3 give bus 1 device 3
/// the object 0.
#[test]
fn open_device_sync_term_keys_each_device_by_its_bus_and_device_order() {
    let lane = |object, field, value| contribution(LaneIndex::new(176, object, field, 0), value);
    let sum = [
        lane(0, 0, 1),
        lane(0, 1, 1),
        lane(0, 2, 3),
        lane(1, 0, 1),
        lane(1, 1, 2),
        lane(1, 2, 5),
    ]
    .into_iter()
    .fold(0u128, u128::wrapping_add);
    let later = OpenDevice { bus: 1, device: 3 };
    let first = OpenDevice { bus: 2, device: 5 };
    assert_eq!(state(&[first, later]).sync_term(), sum);
    let order: Vec<_> = state(&[first, later]).open_devices().collect();
    assert_eq!(order, [later, first]);
}

#[test]
fn each_device_and_each_field_moves_the_term() {
    let one = state(&[BUS_1_DEVICE_0]).sync_term();
    assert_ne!(one, 0);
    let other = OpenDevice { bus: 1, device: 1 };
    assert_ne!(state(&[BUS_1_DEVICE_0, other]).sync_term(), one);
    let changed = [
        OpenDevice { bus: 2, device: 0 },
        OpenDevice { bus: 1, device: 1 },
    ];
    assert_eq!(changed.len(), 2);
    for device in changed {
        assert_ne!(state(&[device]).sync_term(), one, "{device:?}");
    }
}

#[test]
fn a_second_open_of_one_device_leaves_the_state_and_term() {
    let once = state(&[BUS_1_DEVICE_0]);
    let twice = state(&[BUS_1_DEVICE_0, BUS_1_DEVICE_0]);
    assert_eq!(twice, once);
    assert_eq!(twice.sync_term(), once.sync_term());
    assert!(once.is_open(BUS_1_DEVICE_0));
    assert!(!once.is_open(OpenDevice { bus: 1, device: 1 }));
}

/// The stamp is left to the scheme guard: the checkpoint scheme move
/// stamps each pending surface with its new version.
#[test]
fn the_group_surface_is_on_its_own_lane_source() {
    assert_eq!(GROUP.hashed_surfaces.len(), 1);
    let surface = GROUP.hashed_surfaces[0];
    assert_eq!(surface.source, Some(OPEN_DEVICE_SOURCE));
    assert_eq!(OPEN_DEVICE_SOURCE, 176);
}
