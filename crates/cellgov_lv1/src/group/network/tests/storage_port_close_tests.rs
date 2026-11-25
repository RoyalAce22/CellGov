use super::*;
use crate::group::device_bus::{DeviceBusState, OpenDevice, StorageBusOpen, StorageDeviceOpen};
use crate::group::interrupt::event_port::{EventReceivePort, EventReceivePortState};
use crate::group::memory::address_space::AddressSpaceState;
use crate::group::network::interrupt_port::connect_port;
use crate::group::network::NetworkState;
use crate::group::spe::LogicalSpeState;
use cellgov_event::UnitId;
use cellgov_ps3_abi::lv1::call::CONNECT_INTERRUPT_EVENT_RECEIVE_PORT;
use cellgov_ps3_abi::lv1::device_bus::STORAGE_BUS;
use cellgov_time::GuestTicks;

/// The port that the state connects before the close.
const HELD_PORT: u64 = 21;

/// A second port of the hypervisor.
const OTHER_PORT: u64 = 22;

/// The call for device `device` of bus 4 with the port `port` and r6 0.
fn request(device: u64, port: u64) -> HypercallRequest {
    let mut inputs = [0; 8];
    inputs[..4].copy_from_slice(&[STORAGE_BUS, device, port, 0]);
    HypercallRequest {
        unit: UnitId::new(0),
        number: CONNECT_INTERRUPT_EVENT_RECEIVE_PORT,
        inputs,
        pc: 0,
        lr: 0,
        thread: Some(0),
        problem_state: false,
        tick: GuestTicks::new(0),
    }
}

/// The storage bus open with [`HELD_PORT`] connected to its slot, or to
/// the slot of storage device 0 that is open too.
fn connected(device: u64) -> DeviceBusState {
    let mut devices = DeviceBusState::default();
    devices.apply(DeviceBusChange::StorageBusOpened(StorageBusOpen { r5: 0 }));
    if device == STORAGE_BUS_DEVICE {
        devices.apply(DeviceBusChange::StorageBusPortConnected(HELD_PORT));
        return devices;
    }
    devices.apply(DeviceBusChange::StorageDeviceOpened(StorageDeviceOpen {
        device,
        r5: 0,
    }));
    devices.apply(DeviceBusChange::StorageDevicePortConnected(
        StorageDevicePort {
            device,
            port: HELD_PORT,
        },
    ));
    devices
}

fn connect(devices: &DeviceBusState, call: &HypercallRequest) -> HypercallDisposition {
    let mut ports = EventReceivePortState::default();
    for id in [HELD_PORT, OTHER_PORT] {
        ports.construct(EventReceivePort::new(id));
    }
    connect_port(
        call,
        Some("4.93"),
        &PortState {
            devices,
            network: &NetworkState::default(),
            ports: &ports,
            spaces: &AddressSpaceState::default(),
            spes: &LogicalSpeState::default(),
        },
    )
}

fn answered(r3: u64) -> HypercallDisposition {
    HypercallDisposition::Answered(HypercallAnswer::lv1_status(r3, [0; 7]))
}

/// [OFW-4.93 lv1 a:0x255520-0x255548] gives -9 while the slot's handle
/// holds a port. After the close, the access check finds no used slot of
/// the partition [OFW-4.93 lv1 a:0x25477C-0x2547FC] and gives -11
/// [OFW-4.93 lv1 a:0x236534-0x236538].
#[test]
fn a_storage_device_connect_after_the_close_change_gets_wrong_state_where_it_got_busy() {
    let mut devices = connected(0);
    for port in [HELD_PORT, OTHER_PORT] {
        assert_eq!(
            connect(&devices, &request(0, port)),
            answered(STORAGE_PORT_BUSY_R3),
            "port {port} before the close"
        );
    }
    devices.apply(DeviceBusChange::Closed(OpenDevice {
        bus: STORAGE_BUS,
        device: 0,
    }));
    for port in [HELD_PORT, OTHER_PORT] {
        assert_eq!(
            connect(&devices, &request(0, port)),
            answered(STORAGE_PORT_NOT_OPEN_R3),
            "port {port} after the close"
        );
    }
}

/// [OFW-4.93 lv1 a:0x255520-0x255548] gives -9 while the slot's handle
/// holds a port. After the close, the partition holds no slot of the
/// storage bus [OFW-4.93 lv1 a:0x25477C-0x2547FC] and gets -11
/// [OFW-4.93 lv1 a:0x236534-0x236538].
#[test]
fn a_storage_bus_connect_after_the_close_change_gets_wrong_state_where_it_got_busy() {
    let mut devices = connected(STORAGE_BUS_DEVICE);
    assert_eq!(
        connect(&devices, &request(STORAGE_BUS_DEVICE, OTHER_PORT)),
        answered(STORAGE_PORT_BUSY_R3)
    );
    devices.apply(DeviceBusChange::Closed(OpenDevice {
        bus: STORAGE_BUS,
        device: STORAGE_BUS_DEVICE,
    }));
    assert_eq!(
        connect(&devices, &request(STORAGE_BUS_DEVICE, OTHER_PORT)),
        answered(STORAGE_PORT_NOT_OPEN_R3)
    );
}
