use super::*;
use crate::answer::HypercallRefusal;
use crate::group::device_bus::{DeviceBusState, StorageBusOpen, StorageDeviceOpen};
use crate::group::interrupt::event_port::{EventReceivePort, EventReceivePortState};
use crate::group::memory::address_space::AddressSpaceState;
use crate::group::network::interrupt_port::{connect_port, OUTLET, UNKNOWN_ID};
use crate::group::network::NetworkState;
use crate::group::spe::LogicalSpeState;
use crate::Hypervisor;
use cellgov_event::UnitId;
use cellgov_ps3_abi::lv1::call::{
    CONNECT_INTERRUPT_EVENT_RECEIVE_PORT, CONSTRUCT_EVENT_RECEIVE_PORT, OPEN_DEVICE,
};
use cellgov_time::GuestTicks;

/// The `sc` of the 4.93 kernel's call in its port setup
/// [OFW-4.93 lv2_kernel a:0x80000000000964CC].
const KERNEL_SC: u64 = 0x8000_0000_0009_64CC;

/// The `sc` of the port setup's `construct_event_receive_port`
/// [OFW-4.93 lv2_kernel a:0x8000000000096494].
const KERNEL_PORT_SC: u64 = 0x8000_0000_0009_6494;

/// The r5 of the kernel's open of a storage device
/// [OFW-4.93 lv2_kernel a:0x80000000000ac0fc-0x80000000000ac118].
const KERNEL_OPEN_R5: u64 = 0x8000_0000_0000_0000;

/// The port id that the console capture `lv1_storage_device_port_ids`
/// reads in the game OS slot of device 0.
const DEVICE_0_PORT: u64 = 21;

/// A second port of the hypervisor.
const OTHER_PORT: u64 = 22;

/// The logical PPE id of the 4.93 GameOS partition.
const PPE_ID: u64 = 1;

/// An irq outlet id that no port holds.
const OUTLET_ID: u64 = 12;

/// An id that names no object of the hypervisor.
const UNKNOWN: u64 = 0x41;

/// The call for device `device` of bus 4, with the port `port` and `r6`.
fn request(device: u64, port: u64, r6: u64) -> HypercallRequest {
    let mut inputs = [0; 8];
    inputs[..4].copy_from_slice(&[4, device, port, r6]);
    HypercallRequest {
        unit: UnitId::new(0),
        number: CONNECT_INTERRUPT_EVENT_RECEIVE_PORT,
        inputs,
        pc: KERNEL_SC,
        lr: 0,
        thread: Some(0),
        problem_state: false,
        tick: GuestTicks::new(35_495),
    }
}

/// The hypervisor's state at a storage device's call.
struct Held {
    devices: DeviceBusState,
    ports: EventReceivePortState,
}

impl Held {
    /// The state with the storage bus open, each device of `opened` open
    /// with the kernel's r5, two ports and an irq outlet.
    fn opened(opened: &[u64]) -> Self {
        let mut devices = DeviceBusState::default();
        devices.apply(DeviceBusChange::StorageBusOpened(StorageBusOpen { r5: 0 }));
        for &device in opened {
            devices.apply(DeviceBusChange::StorageDeviceOpened(StorageDeviceOpen {
                device,
                r5: KERNEL_OPEN_R5,
            }));
        }
        let mut ports = EventReceivePortState::default();
        for id in [DEVICE_0_PORT, OTHER_PORT] {
            ports.construct(EventReceivePort::new(id));
        }
        ports.construct_outlet(EventReceivePort::new(OUTLET_ID));
        Self { devices, ports }
    }

    /// The state with device 0 open and its port connected.
    fn connected() -> Self {
        let mut held = Self::opened(&[0]);
        held.devices
            .apply(DeviceBusChange::StorageDevicePortConnected(
                StorageDevicePort {
                    device: 0,
                    port: DEVICE_0_PORT,
                },
            ));
        held
    }

    fn connect(&self, r: &HypercallRequest) -> HypercallDisposition {
        connect_port(
            r,
            Some("4.93"),
            &PortState {
                devices: &self.devices,
                network: &NetworkState::default(),
                ports: &self.ports,
                spaces: &AddressSpaceState::default(),
                spes: &LogicalSpeState::default(),
            },
        )
    }
}

fn answered(r3: u64) -> HypercallDisposition {
    HypercallDisposition::Answered(HypercallAnswer::lv1_status(r3, [0; 7]))
}

fn connected(device: u64, port: u64) -> HypercallDisposition {
    let mut answer = HypercallAnswer::lv1_status(0, [0; 7]);
    answer.push_change(DeviceBusChange::StorageDevicePortConnected(
        StorageDevicePort { device, port },
    ));
    HypercallDisposition::Answered(answer)
}

fn refused(request: &HypercallRequest, what: &'static str) -> HypercallDisposition {
    HypercallDisposition::Refused(HypercallRefusal::UnmodelledInput {
        number: CONNECT_INTERRUPT_EVENT_RECEIVE_PORT,
        unit: request.unit,
        pc: request.pc,
        what,
    })
}

fn r3_of(disposition: &HypercallDisposition) -> Option<u64> {
    match disposition {
        HypercallDisposition::Answered(answer) => Some(answer.r3),
        HypercallDisposition::Refused(_) => None,
    }
}

/// [OFW-4.93 lv1 a:0x23699C-0x2369BC] gives the device object to the
/// storage connect, which links the port and returns 0
/// [OFW-4.93 lv1 a:0x25554C-0x255570] [OFW-4.93 lv1 a:0x2555E8-0x2555F4].
#[test]
fn each_open_device_of_the_bus_table_connects_its_port_with_r3_0_and_r4_to_r10_cleared() {
    let devices = [0, 1, 2, 3, 4];
    let held = Held::opened(&devices);
    for device in devices {
        assert_eq!(
            held.connect(&request(device, DEVICE_0_PORT, 0)),
            connected(device, DEVICE_0_PORT),
            "device {device}"
        );
    }
}

/// [OFW-4.93 lv1 a:0x25477C-0x2547FC] finds no slot of the partition in
/// the device object, so the access check gives -11 before the r6 check
/// and the storage connect.
#[test]
fn a_device_that_the_partition_did_not_open_gets_wrong_state_while_the_storage_bus_is_open() {
    let held = Held::opened(&[1]);
    assert!(held.devices.storage_bus_open().is_some());
    for (port, r6) in [(DEVICE_0_PORT, 0), (UNKNOWN, 1)] {
        assert_eq!(
            held.connect(&request(0, port, r6)),
            answered(STORAGE_PORT_NOT_OPEN_R3),
            "port {port:#x}, r6 {r6}"
        );
    }
}

/// [OFW-4.93 lv1 a:0x236994-0x236998] checks all 64 bits of r6 before the
/// storage connect, whose port and busy checks come after it.
#[test]
fn a_storage_device_call_with_an_r6_other_than_0_gets_r6_marked_before_the_port_check() {
    for r6 in [1, 1 << 32, u64::MAX] {
        assert_eq!(
            Held::opened(&[0]).connect(&request(0, UNKNOWN, r6)),
            answered(STORAGE_PORT_R6_R3),
            "r6 {r6:#x}"
        );
        assert_eq!(
            Held::connected().connect(&request(0, OTHER_PORT, r6)),
            answered(STORAGE_PORT_R6_R3),
            "r6 {r6:#x} while a port holds the slot"
        );
    }
}

/// [OFW-4.93 lv1 a:0x2553B8-0x2553D0] checks the port class before the
/// busy check at [OFW-4.93 lv1 a:0x255520-0x255548].
#[test]
fn a_storage_device_call_with_the_ppe_id_gets_r5_marked_also_while_a_port_holds_the_slot() {
    for held in [Held::opened(&[0]), Held::connected()] {
        assert_eq!(
            held.connect(&request(0, PPE_ID, 0)),
            answered(STORAGE_PORT_NO_PORT_R3)
        );
    }
}

/// [OFW-4.93 lv1 a:0x255520-0x255548] returns -9 while the handle of the
/// device's slot holds a port with a reference.
#[test]
fn a_device_whose_slot_holds_a_port_gets_busy_and_another_open_device_connects() {
    let mut held = Held::connected();
    for id in [DEVICE_0_PORT, OTHER_PORT] {
        assert_eq!(
            held.connect(&request(0, id, 0)),
            answered(STORAGE_PORT_BUSY_R3),
            "port {id}"
        );
    }
    held.devices
        .apply(DeviceBusChange::StorageDeviceOpened(StorageDeviceOpen {
            device: 1,
            r5: KERNEL_OPEN_R5,
        }));
    assert_eq!(
        held.connect(&request(1, OTHER_PORT, 0)),
        connected(1, OTHER_PORT)
    );
}

#[test]
fn a_device_outside_the_table_an_irq_outlet_or_an_unknown_id_for_a_device_is_a_refusal() {
    let held = Held::opened(&[0]);
    let cases = [
        (request(5, DEVICE_0_PORT, 0), STORAGE_DEVICE.what),
        (request(6, DEVICE_0_PORT, 0), STORAGE_DEVICE.what),
        (request(0, OUTLET_ID, 0), OUTLET.what),
        (request(0, UNKNOWN, 0), UNKNOWN_ID.what),
    ];
    assert_eq!(cases.len(), 4);
    for (call, what) in cases {
        assert_eq!(held.connect(&call), refused(&call, what), "{call:?}");
    }
}

#[test]
fn a_device_call_at_a_firmware_with_no_bus_table_is_a_refusal() {
    let held = Held::opened(&[0]);
    let call = request(0, DEVICE_0_PORT, 0);
    let state = PortState {
        devices: &held.devices,
        network: &NetworkState::default(),
        ports: &held.ports,
        spaces: &AddressSpaceState::default(),
        spes: &LogicalSpeState::default(),
    };
    assert_eq!(
        connect_storage_port(&call, "4.92", &state),
        refused(&call, STORAGE_DEVICE.what)
    );
}

#[test]
fn the_hypervisor_connects_a_storage_device_port_after_its_open_and_then_gets_busy() {
    let mut hv = Hypervisor::new();
    hv.set_firmware("4.93");
    let open = HypercallRequest {
        number: OPEN_DEVICE,
        ..request(3, KERNEL_OPEN_R5, 0)
    };
    assert_eq!(r3_of(&hv.dispatch(&open)), Some(0), "open_device");
    let construct = HypercallRequest {
        number: CONSTRUCT_EVENT_RECEIVE_PORT,
        pc: KERNEL_PORT_SC,
        ..request(0, 0, 0)
    };
    let port = match hv.dispatch(&construct) {
        HypercallDisposition::Answered(made) => {
            assert_eq!(made.r3, 0, "construct_event_receive_port");
            made.outputs().expect("a table return")[0]
        }
        HypercallDisposition::Refused(refusal) => panic!("{refusal:?}"),
    };
    let connect = request(3, port, 0);
    let before = hv.sync_term();
    assert_eq!(r3_of(&hv.dispatch(&connect)), Some(0));
    assert_eq!(
        hv.groups().device_bus().storage_device_port(3),
        Some(StorageDevicePort { device: 3, port })
    );
    assert_eq!(hv.groups().device_bus().storage_bus_port(), None);
    assert_ne!(hv.sync_term(), before);
    let after_first = hv.clone();
    assert_eq!(r3_of(&hv.dispatch(&connect)), Some(STORAGE_PORT_BUSY_R3));
    assert_eq!(hv, after_first);
}
