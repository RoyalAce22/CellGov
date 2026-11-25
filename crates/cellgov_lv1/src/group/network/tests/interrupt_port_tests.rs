use super::*;
use crate::answer::HypercallRefusal;
use crate::group::device_bus::{DeviceBusChange, OpenDevice};
use crate::group::interrupt::event_port::EventReceivePort;
use crate::group::spe::LogicalSpe;
use crate::{answered_numbers, AddressSpace, AddressSpaceChange, Hypervisor};
use cellgov_event::UnitId;
use cellgov_ps3_abi::lv1::call::{CONSTRUCT_EVENT_RECEIVE_PORT, OPEN_DEVICE};
use cellgov_time::GuestTicks;

/// The `sc` of the 4.93 kernel's call
/// [OFW-4.93 lv2_kernel a:0x80000000000A2930].
const KERNEL_SC: u64 = 0x8000_0000_000A_2930;

/// The `sc` of the kernel's `construct_event_receive_port` that makes the
/// port of its call [OFW-4.93 lv2_kernel a:0x80000000000A28E0].
const KERNEL_PORT_SC: u64 = 0x8000_0000_000A_28E0;

/// The port id of the kernel's call: the id that the console capture
/// `tests/micro/lv1_network_port_id` reads for the port lv1 connected.
const KERNEL_PORT: u64 = 19;

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
const UNKNOWN: u64 = 0x40;

/// The call for device `device` of bus `bus`, with the port `port` and
/// `r6`.
fn request(bus: u64, device: u64, port: u64, r6: u64) -> HypercallRequest {
    let mut inputs = [0; 8];
    inputs[..4].copy_from_slice(&[bus, device, port, r6]);
    HypercallRequest {
        unit: UnitId::new(0),
        number: CONNECT_INTERRUPT_EVENT_RECEIVE_PORT,
        inputs,
        pc: KERNEL_SC,
        lr: 0,
        thread: Some(0),
        problem_state: false,
        tick: GuestTicks::new(32_908),
    }
}

/// The kernel's call: bus 1, device 0, its port and r6 = 0
/// [OFW-4.93 lv2_kernel a:0x80000000000A291C-0x80000000000A292C].
fn kernel_call() -> HypercallRequest {
    request(1, 0, KERNEL_PORT, 0)
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
    /// The state with the network device open, the kernel's port, an irq
    /// outlet, the boot vas and the boot SPE.
    fn boot() -> Self {
        let mut devices = DeviceBusState::default();
        devices.apply(DeviceBusChange::Opened(NETWORK_DEVICE));
        let mut ports = EventReceivePortState::default();
        ports.construct(EventReceivePort::new(KERNEL_PORT));
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
        held.network
            .apply(NetworkChange::InterruptPortConnected(KERNEL_PORT));
        held
    }

    fn connect(&self, r: &HypercallRequest) -> HypercallDisposition {
        self.connect_at(r, Some("4.93"))
    }

    fn connect_at(&self, r: &HypercallRequest, firmware: Option<&str>) -> HypercallDisposition {
        connect_port(
            r,
            firmware,
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

/// [OFW-4.93 lv1 a:0x246140-0x246150] links the port;
/// [OFW-4.93 lv1 a:0x2462F4] returns 0.
#[test]
fn the_kernel_call_gets_r3_0_with_r4_to_r10_cleared_and_connects_its_port() {
    let mut expected = HypercallAnswer::lv1_status(0, [0; 7]);
    expected.push_change(NetworkChange::InterruptPortConnected(KERNEL_PORT));
    assert_eq!(
        Held::boot().connect(&kernel_call()),
        HypercallDisposition::Answered(expected)
    );
}

/// [OFW-4.93 lv1 a:0x245F88-0x245FB0] returns -9 before the port lookup
/// at [OFW-4.93 lv1 a:0x245FFC].
#[test]
fn a_call_while_a_port_holds_the_interrupt_gets_busy_for_any_port_id() {
    let held = Held::connected();
    let ids = [KERNEL_PORT, OUTLET_ID, UNKNOWN, VAS_ID];
    assert_eq!(ids.len(), 4);
    for id in ids {
        assert_eq!(
            held.connect(&request(1, 0, id, 0)),
            answered(INTERRUPT_PORT_BUSY_R3),
            "port {id:#x}"
        );
    }
}

/// [OFW-4.93 lv1 a:0x305D88-0x305D8C] checks the range in the handler
/// over all 64 bits, so a bus id with a high word is out of range;
/// [OFW-4.93 lv1 a:0x2363F0-0x236428] refuses bus 0 before the open check.
#[test]
fn each_failed_bus_check_gets_its_marked_status_before_the_open_check() {
    let mut held = Held::boot();
    held.devices = DeviceBusState::default();
    let cases = [
        (5, INTERRUPT_PORT_BUS_RANGE_R3),
        ((1 << 32) | 1, INTERRUPT_PORT_BUS_RANGE_R3),
        (u64::MAX, INTERRUPT_PORT_BUS_RANGE_R3),
        (0, INTERRUPT_PORT_NO_BUS_R3),
    ];
    assert_eq!(cases.len(), 4);
    for (bus, r3) in cases {
        assert_eq!(
            held.connect(&request(bus, 0, KERNEL_PORT, 1)),
            answered(r3),
            "bus {bus:#x}"
        );
    }
}

/// [OFW-4.93 lv1 a:0x2364B8-0x236538] gives -11 before the r6 check at
/// [OFW-4.93 lv1 a:0x236A14] and the port connect function.
#[test]
fn a_call_before_the_partition_opened_the_network_device_gets_wrong_state() {
    let mut other = DeviceBusState::default();
    other.apply(DeviceBusChange::Opened(OpenDevice { bus: 1, device: 1 }));
    for devices in [DeviceBusState::default(), other] {
        let mut held = Held::connected();
        held.devices = devices;
        for r6 in [0, 1] {
            assert_eq!(
                held.connect(&request(1, 0, UNKNOWN, r6)),
                answered(INTERRUPT_PORT_NOT_OPEN_R3),
                "r6 {r6}"
            );
        }
    }
}

/// [OFW-4.93 lv1 a:0x236A14-0x236A18] checks all 64 bits of r6 before
/// the port connect function's busy check.
#[test]
fn an_r6_other_than_0_gets_r6_marked_before_the_busy_check() {
    for r6 in [1, 1 << 32, u64::MAX] {
        assert_eq!(
            Held::connected().connect(&request(1, 0, KERNEL_PORT, r6)),
            answered(INTERRUPT_PORT_R6_R3),
            "r6 {r6:#x}"
        );
    }
}

/// [OFW-4.93 lv1 a:0x246114-0x246120] checks the port class;
/// [OFW-4.93 lv1 a:0x236BEC-0x236BF8] marks r5.
#[test]
fn an_id_of_an_object_of_another_class_gets_r5_marked_and_no_change() {
    let held = Held::boot();
    let ids = [PPE_ID, VAS_ID, BOOT_SPE_ID];
    assert_eq!(ids.len(), 3);
    for id in ids {
        assert_eq!(
            held.connect(&request(1, 0, id, 0)),
            answered(INTERRUPT_PORT_NO_PORT_R3),
            "id {id}"
        );
    }
}

#[test]
fn a_call_for_another_bus_or_device_is_a_refusal() {
    let held = Held::boot();
    let cases = [
        (request(2, 0, KERNEL_PORT, 0), OTHER_BUS.what),
        (request(3, 0, KERNEL_PORT, 0), OTHER_BUS.what),
        (request(4, 0, KERNEL_PORT, 0), OTHER_BUS.what),
        (request(1, 1, KERNEL_PORT, 0), OTHER_DEVICE.what),
        (request(1, 1 << 32, KERNEL_PORT, 0), OTHER_DEVICE.what),
    ];
    assert_eq!(cases.len(), 5);
    for (call, what) in cases {
        assert_eq!(held.connect(&call), refused(&call, what), "{call:?}");
    }
}

#[test]
fn an_irq_outlet_or_an_unknown_id_is_a_refusal() {
    let held = Held::boot();
    let outlet = request(1, 0, OUTLET_ID, 0);
    assert_eq!(held.connect(&outlet), refused(&outlet, OUTLET.what));
    let unknown = request(1, 0, UNKNOWN, 0);
    assert_eq!(held.connect(&unknown), refused(&unknown, UNKNOWN_ID.what));
}

/// The object table lookup reads all 64 bits of the id
/// [OFW-4.93 lv1 a:0x246040] [OFW-4.93 lv1 a:0x2460B4], so a high word
/// above the kernel's port id does not name that port.
#[test]
fn a_port_id_with_a_high_word_does_not_name_the_port_of_its_low_word() {
    let held = Held::boot();
    let dirty = request(1, 0, (1 << 32) | KERNEL_PORT, 0);
    assert_eq!(held.connect(&dirty), refused(&dirty, UNKNOWN_ID.what));
}

#[test]
fn a_call_at_a_firmware_whose_handler_is_not_read_is_a_refusal() {
    let call = kernel_call();
    for firmware in [None, Some("3.70"), Some("4.92")] {
        assert_eq!(
            Held::boot().connect_at(&call, firmware),
            refused(&call, UNREAD_FIRMWARE.what),
            "{firmware:?}"
        );
    }
}

#[test]
fn the_hypervisor_answers_197_after_the_open_and_the_port_then_gets_busy() {
    assert!(answered_numbers().contains(&CONNECT_INTERRUPT_EVENT_RECEIVE_PORT));
    let mut hv = Hypervisor::new();
    hv.set_firmware("4.93");
    let open = HypercallRequest {
        number: OPEN_DEVICE,
        ..request(1, 0, 0, 0)
    };
    assert_eq!(r3_of(&hv.dispatch(&open)), Some(0));
    let construct = HypercallRequest {
        number: CONSTRUCT_EVENT_RECEIVE_PORT,
        pc: KERNEL_PORT_SC,
        ..request(0, 0, 0, 0)
    };
    let port = match hv.dispatch(&construct) {
        HypercallDisposition::Answered(made) => {
            assert_eq!(made.r3, 0, "construct_event_receive_port");
            made.outputs().expect("a table return")[0]
        }
        HypercallDisposition::Refused(refusal) => panic!("{refusal:?}"),
    };
    assert!(hv.groups().interrupt().ports().port(port).is_some());
    let connect = request(1, 0, port, 0);
    let before = hv.sync_term();
    assert_eq!(r3_of(&hv.dispatch(&connect)), Some(0));
    assert_eq!(hv.groups().network().interrupt_port(), Some(port));
    assert_ne!(hv.sync_term(), before);
    let after_first = hv.clone();
    assert_eq!(r3_of(&hv.dispatch(&connect)), Some(INTERRUPT_PORT_BUSY_R3));
    assert_eq!(hv, after_first);
}
