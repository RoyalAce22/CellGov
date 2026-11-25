use super::*;
use crate::answer::HypercallRefusal;
use crate::group::device_bus::{DeviceBusState, StorageBusOpen};
use crate::group::interrupt::event_port::{EventReceivePort, EventReceivePortState};
use crate::group::memory::address_space::AddressSpaceState;
use crate::group::network::interrupt_port::{connect_port, OUTLET, UNKNOWN_ID};
use crate::group::network::NetworkState;
use crate::group::spe::{LogicalSpe, LogicalSpeState};
use crate::{answered_numbers, AddressSpace, AddressSpaceChange, Hypervisor};
use cellgov_event::UnitId;
use cellgov_ps3_abi::lv1::call::{
    CONNECT_INTERRUPT_EVENT_RECEIVE_PORT, CONSTRUCT_EVENT_RECEIVE_PORT, OPEN_DEVICE,
};
use cellgov_ps3_abi::lv1::network::STORAGE_PORT_ID_LIMIT;
use cellgov_time::GuestTicks;

/// The `sc` of the 4.93 kernel's call in its port setup
/// [OFW-4.93 lv2_kernel a:0x80000000000964CC].
const KERNEL_SC: u64 = 0x8000_0000_0009_64CC;

/// The `sc` of the port setup's `construct_event_receive_port`
/// [OFW-4.93 lv2_kernel a:0x8000000000096494].
const KERNEL_PORT_SC: u64 = 0x8000_0000_0009_6494;

/// The port id of the kernel's call: the id that the console capture
/// `tests/micro/lv1_storage_port_id` reads for the port lv1 connected.
const KERNEL_PORT: u64 = 20;

/// A second port of the hypervisor.
const OTHER_PORT: u64 = 21;

/// A port id outside segment 0 of the object table.
const HIGH_PORT: u64 = STORAGE_PORT_ID_LIMIT;

/// The logical PPE id of the 4.93 GameOS partition.
const PPE_ID: u64 = 1;

/// The vas id that the 4.93 kernel's boot `construct_virtual_address_space`
/// gets.
const VAS_ID: u64 = 2;

/// The spe id of the 4.93 kernel's boot SPE in the capture
/// `lv2_logical_spe_object`.
const BOOT_SPE_ID: u64 = 11;

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
        tick: GuestTicks::new(33_915),
    }
}

/// The kernel's call: bus 4, device -1, its port and r6 = 0
/// [OFW-4.93 lv2_kernel a:0x80000000000964B8-0x80000000000964C8].
fn kernel_call() -> HypercallRequest {
    request(u64::MAX, KERNEL_PORT, 0)
}

/// The hypervisor's state at the kernel's call.
struct Held {
    devices: DeviceBusState,
    network: NetworkState,
    ports: EventReceivePortState,
    spaces: AddressSpaceState,
    spes: LogicalSpeState,
}

impl Held {
    /// The state with the storage bus open, three ports, an irq outlet,
    /// the boot vas and the boot SPE.
    fn boot() -> Self {
        let mut devices = DeviceBusState::default();
        devices.apply(DeviceBusChange::StorageBusOpened(StorageBusOpen { r5: 0 }));
        let mut ports = EventReceivePortState::default();
        for id in [KERNEL_PORT, OTHER_PORT, HIGH_PORT] {
            ports.construct(EventReceivePort::new(id));
        }
        ports.construct_outlet(EventReceivePort::new(OUTLET_ID));
        let mut spaces = AddressSpaceState::default();
        spaces.apply(AddressSpaceChange::Constructed(AddressSpace {
            vas_id: VAS_ID,
            htab_size_exp: 18,
            page_size_count: 2,
            page_shifts: 0,
            mapping: None,
        }));
        let mut spes = LogicalSpeState::default();
        spes.construct(LogicalSpe {
            id: BOOT_SPE_ID,
            spe_type: 1,
            vas_id: VAS_ID,
            area: 0x0000_4c00_0028_0000,
            shadow: 0x0000_3000_0000_6000,
            shadow_real: 0x1100_0000,
        });
        Self {
            devices,
            network: NetworkState::default(),
            ports,
            spaces,
            spes,
        }
    }

    /// The boot state after the kernel's call connected its port.
    fn connected() -> Self {
        let mut held = Self::boot();
        held.devices
            .apply(DeviceBusChange::StorageBusPortConnected(KERNEL_PORT));
        held
    }

    fn connect(&self, r: &HypercallRequest) -> HypercallDisposition {
        connect_port(
            r,
            Some("4.93"),
            &PortState {
                devices: &self.devices,
                network: &self.network,
                ports: &self.ports,
                spaces: &self.spaces,
                spes: &self.spes,
            },
        )
    }
}

fn answered(r3: u64) -> HypercallDisposition {
    HypercallDisposition::Answered(HypercallAnswer::lv1_status(r3, [0; 7]))
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

/// [OFW-4.93 lv1 a:0x25554C-0x255570] links the port;
/// [OFW-4.93 lv1 a:0x2555E8-0x2555F4] returns 0.
#[test]
fn the_kernel_call_gets_r3_0_with_r4_to_r10_cleared_and_connects_its_port() {
    let mut expected = HypercallAnswer::lv1_status(0, [0; 7]);
    expected.push_change(DeviceBusChange::StorageBusPortConnected(KERNEL_PORT));
    assert_eq!(
        Held::boot().connect(&kernel_call()),
        HypercallDisposition::Answered(expected)
    );
}

/// [OFW-4.93 lv1 a:0x2364E8-0x2364F4] [OFW-4.93 lv1 a:0x236554-0x236558]
/// find no device before the open check.
#[test]
fn a_device_above_6_other_than_minus_1_gets_r4_marked_before_the_open_check() {
    let mut held = Held::boot();
    held.devices = DeviceBusState::default();
    let devices = [7, 0xFFFF_FFFF, u64::MAX - 1];
    assert_eq!(devices.len(), 3);
    for device in devices {
        assert_eq!(
            held.connect(&request(device, UNKNOWN, 1)),
            answered(STORAGE_PORT_NO_DEVICE_R3),
            "device {device:#x}"
        );
    }
}

/// [OFW-4.93 lv1 a:0x25477C-0x2547FC] finds no slot of the partition, so
/// the access check gives -11 before the r6 check and the storage connect.
#[test]
fn a_call_before_the_partition_opened_the_storage_bus_gets_wrong_state() {
    let mut held = Held::connected();
    held.devices = DeviceBusState::default();
    for (port, r6) in [(KERNEL_PORT, 0), (UNKNOWN, 1)] {
        assert_eq!(
            held.connect(&request(u64::MAX, port, r6)),
            answered(STORAGE_PORT_NOT_OPEN_R3),
            "port {port:#x}, r6 {r6}"
        );
    }
}

/// [OFW-4.93 lv1 a:0x236994-0x236998] checks all 64 bits of r6 before the
/// storage connect.
#[test]
fn an_r6_other_than_0_gets_r6_marked_before_the_storage_connect() {
    for r6 in [1, 1 << 32, u64::MAX] {
        assert_eq!(
            Held::connected().connect(&request(u64::MAX, UNKNOWN, r6)),
            answered(STORAGE_PORT_R6_R3),
            "r6 {r6:#x}"
        );
    }
}

/// [OFW-4.93 lv1 a:0x2553B8-0x2553D0] checks the port class before the
/// busy check at [OFW-4.93 lv1 a:0x255520-0x255548].
#[test]
fn an_id_of_another_class_gets_r5_marked_also_while_a_port_holds_the_slot() {
    let ids = [PPE_ID, VAS_ID, BOOT_SPE_ID];
    assert_eq!(ids.len(), 3);
    for held in [Held::boot(), Held::connected()] {
        for id in ids {
            assert_eq!(
                held.connect(&request(u64::MAX, id, 0)),
                answered(STORAGE_PORT_NO_PORT_R3),
                "id {id}"
            );
        }
    }
}

/// [OFW-4.93 lv1 a:0x255348-0x2553B4] finds an id in each segment, and
/// [OFW-4.93 lv1 a:0x2554E8-0x2554EC] tests only the lookup's null flag.
#[test]
fn a_port_outside_segment_0_connects() {
    let held = Held::boot();
    assert!(held.ports.port(HIGH_PORT).is_some());
    let mut expected = HypercallAnswer::lv1_status(0, [0; 7]);
    expected.push_change(DeviceBusChange::StorageBusPortConnected(HIGH_PORT));
    assert_eq!(
        held.connect(&request(u64::MAX, HIGH_PORT, 0)),
        HypercallDisposition::Answered(expected)
    );
}

/// [OFW-4.93 lv1 a:0x255520-0x255548] returns -9 while the slot's handle
/// holds a port with a reference.
#[test]
fn a_call_while_a_port_holds_the_slot_gets_busy() {
    let held = Held::connected();
    for id in [KERNEL_PORT, OTHER_PORT] {
        assert_eq!(
            held.connect(&request(u64::MAX, id, 0)),
            answered(STORAGE_PORT_BUSY_R3),
            "port {id}"
        );
    }
}

#[test]
fn a_device_outside_the_storage_bus_table_an_irq_outlet_or_an_unknown_id_is_a_refusal() {
    let held = Held::boot();
    let cases = [
        (request(5, KERNEL_PORT, 0), STORAGE_DEVICE.what),
        (request(6, KERNEL_PORT, 0), STORAGE_DEVICE.what),
        (request(u64::MAX, OUTLET_ID, 0), OUTLET.what),
        (request(u64::MAX, UNKNOWN, 0), UNKNOWN_ID.what),
        (
            request(u64::MAX, (1 << 32) | KERNEL_PORT, 0),
            UNKNOWN_ID.what,
        ),
    ];
    assert_eq!(cases.len(), 5);
    for (call, what) in cases {
        assert_eq!(held.connect(&call), refused(&call, what), "{call:?}");
    }
}

#[test]
fn the_hypervisor_connects_the_storage_port_after_the_open_and_then_gets_busy() {
    assert!(answered_numbers().contains(&CONNECT_INTERRUPT_EVENT_RECEIVE_PORT));
    let mut hv = Hypervisor::new();
    hv.set_firmware("4.93");
    let open = HypercallRequest {
        number: OPEN_DEVICE,
        ..request(u64::MAX, 0, 0)
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
    let connect = request(u64::MAX, port, 0);
    let before = hv.sync_term();
    assert_eq!(r3_of(&hv.dispatch(&connect)), Some(0));
    assert_eq!(hv.groups().device_bus().storage_bus_port(), Some(port));
    assert_eq!(hv.groups().network().interrupt_port(), None);
    assert_ne!(hv.sync_term(), before);
    let after_first = hv.clone();
    assert_eq!(r3_of(&hv.dispatch(&connect)), Some(STORAGE_PORT_BUSY_R3));
    assert_eq!(hv, after_first);
}
