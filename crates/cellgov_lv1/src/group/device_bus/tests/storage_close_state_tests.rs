use super::*;
use cellgov_ps3_abi::lv1::device_bus::STORAGE_BUS_DEVICE;

/// The storage bus object: device -1 of bus 4.
const STORAGE_BUS_OBJECT: OpenDevice = OpenDevice {
    bus: STORAGE_BUS,
    device: STORAGE_BUS_DEVICE,
};

/// The port id of the storage bus slot.
const BUS_PORT: u64 = 20;

fn storage_device(device: u64) -> OpenDevice {
    OpenDevice {
        bus: STORAGE_BUS,
        device,
    }
}

/// The state with the storage bus open and its port connected, and each
/// `(device, port)` of `devices` open with its port connected.
fn held(devices: &[(u64, u64)]) -> DeviceBusState {
    let mut state = DeviceBusState::default();
    state.apply(DeviceBusChange::StorageBusOpened(StorageBusOpen { r5: 0 }));
    state.apply(DeviceBusChange::StorageBusPortConnected(BUS_PORT));
    for &(device, port) in devices {
        state.apply(DeviceBusChange::StorageDeviceOpened(StorageDeviceOpen {
            device,
            r5: 0,
        }));
        state.apply(DeviceBusChange::StorageDevicePortConnected(
            StorageDevicePort { device, port },
        ));
    }
    state
}

#[test]
fn a_storage_device_close_removes_its_open_and_port_and_keeps_the_other_device_and_the_bus() {
    let kept = held(&[(1, 22)]);
    let mut state = held(&[(0, 21), (1, 22)]);
    assert_eq!(state.storage_device_opens().count(), 2);
    assert_eq!(state.storage_device_ports().count(), 2);
    state.apply(DeviceBusChange::Closed(storage_device(0)));
    assert_eq!(state.storage_device_open(0), None);
    assert_eq!(state.storage_device_port(0), None);
    assert_eq!(state, kept);
    assert_eq!(state.sync_term(), kept.sync_term());
}

#[test]
fn a_storage_bus_close_removes_the_bus_open_and_its_port() {
    let mut state = held(&[]);
    assert!(state.storage_bus_open().is_some());
    assert_eq!(state.storage_bus_port(), Some(BUS_PORT));
    state.apply(DeviceBusChange::Closed(STORAGE_BUS_OBJECT));
    assert_eq!(state.storage_bus_open(), None);
    assert_eq!(state.storage_bus_port(), None);
    assert_eq!(state, DeviceBusState::default());
    assert_eq!(state.sync_term(), 0);
}

#[test]
fn a_close_of_device_0_of_bus_1_keeps_the_open_and_port_of_storage_device_0() {
    let mut state = held(&[(0, 21)]);
    let kept = state.clone();
    state.apply(DeviceBusChange::Opened(OpenDevice { bus: 1, device: 0 }));
    state.apply(DeviceBusChange::Closed(OpenDevice { bus: 1, device: 0 }));
    assert_eq!(state, kept);
    assert_eq!(
        state.storage_device_port(0),
        Some(StorageDevicePort {
            device: 0,
            port: 21
        })
    );
}
