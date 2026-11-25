use super::*;
use cellgov_mem::lanes::{contribution, LaneIndex};

fn connected(ports: &[(u64, u64)]) -> DeviceBusState {
    let mut state = DeviceBusState::default();
    for &(device, port) in ports {
        state.apply(DeviceBusChange::StorageDevicePortConnected(
            StorageDevicePort { device, port },
        ));
    }
    state
}

/// The list holds every nonzero lane of a state with ports connected to
/// storage devices 1 and 3.
#[test]
fn storage_device_port_sync_term_lane_vector_golden() {
    let lane = |object, field, value| contribution(LaneIndex::new(182, object, field, 0), value);
    let sum = [
        lane(0, 0, 1),
        lane(0, 1, 1),
        lane(0, 2, 22),
        lane(1, 0, 1),
        lane(1, 1, 3),
        lane(1, 2, 24),
    ]
    .into_iter()
    .fold(0u128, u128::wrapping_add);
    assert_eq!(connected(&[(3, 24), (1, 22)]).sync_term(), sum);
}

#[test]
fn the_device_and_the_port_each_move_the_term_and_add_to_the_open_term() {
    let base = connected(&[(0, 21)]).sync_term();
    assert_ne!(base, DeviceBusState::default().sync_term());
    assert_ne!(connected(&[(1, 21)]).sync_term(), base);
    assert_ne!(connected(&[(0, 22)]).sync_term(), base);
    let open = StorageDeviceOpen {
        device: 0,
        r5: 0x8000_0000_0000_0000,
    };
    let mut both = connected(&[(0, 21)]);
    both.apply(DeviceBusChange::StorageDeviceOpened(open));
    let open_term = lanes::value_term(STORAGE_DEVICE_OPEN_SOURCE, 0, &open);
    assert_eq!(both.sync_term(), base.wrapping_add(open_term));
    assert_eq!(
        both.storage_device_port(0),
        Some(StorageDevicePort {
            device: 0,
            port: 21
        })
    );
}

#[test]
fn the_storage_device_port_surface_is_on_its_own_pending_source() {
    let surfaces: Vec<_> = GROUP
        .hashed_surfaces
        .iter()
        .filter(|surface| surface.source == Some(STORAGE_DEVICE_PORT_SOURCE))
        .collect();
    assert_eq!(surfaces.len(), 1);
    assert_eq!(surfaces[0].stamp, Stamp::Pending);
    assert_eq!(STORAGE_DEVICE_PORT_SOURCE, 182);
}
