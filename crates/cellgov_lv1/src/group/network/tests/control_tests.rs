use super::*;
use crate::answer::HypercallRefusal;
use crate::group::device_bus::{DeviceBusChange, OpenDevice};
use crate::{answered_numbers, Hypervisor};
use cellgov_event::UnitId;
use cellgov_ps3_abi::lv1::call::OPEN_DEVICE;
use cellgov_ps3_abi::lv1::network::{
    NET_CONTROL_BUS_RANGE_R3, NET_CONTROL_NO_BUS_R3, NET_CONTROL_NO_DEVICE_R3,
    NET_CONTROL_PRIVILEGED_REQUEST_MAX, NET_CONTROL_REQUEST_MAX, NET_CONTROL_REQUEST_RANGE_R3,
};
use cellgov_time::GuestTicks;

/// The `sc` of the 4.93 kernel's call
/// [OFW-4.93 lv2_kernel a:0x80000000000A369C].
const KERNEL_SC: u64 = 0x8000_0000_000A_369C;

/// A MAC address that differs from the synthetic one.
const OTHER_MAC: u64 = 0x0200_0000_0002;

/// The call for device `device` of bus `bus`, with the request `code`
/// and the inputs `inputs` in r6 to r8.
fn request(bus: u64, device: u64, code: u64, inputs: [u64; 3]) -> HypercallRequest {
    let mut registers = [0; 8];
    registers[..6].copy_from_slice(&[bus, device, code, inputs[0], inputs[1], inputs[2]]);
    HypercallRequest {
        unit: UnitId::new(0),
        number: NET_CONTROL,
        inputs: registers,
        pc: KERNEL_SC,
        lr: 0,
        thread: Some(0),
        problem_state: false,
        tick: GuestTicks::new(32_944),
    }
}

/// The kernel's call: bus 1, device 0, request 1 and 0 in r6 to r8
/// [OFW-4.93 lv2_kernel a:0x80000000000A3680-0x80000000000A3698].
fn kernel_call() -> HypercallRequest {
    request(1, 0, 1, [0; 3])
}

/// The device bus state with the network device open.
fn open() -> DeviceBusState {
    let mut devices = DeviceBusState::default();
    devices.apply(DeviceBusChange::Opened(NETWORK_DEVICE));
    devices
}

fn control(r: &HypercallRequest, devices: &DeviceBusState) -> HypercallDisposition {
    net_control(
        r,
        Some("4.93"),
        devices,
        &SYNTHETIC_CONSOLE_IDENTITY,
        CONSOLE_VLAN_FIELDS,
    )
}

fn answered(r3: u64) -> HypercallDisposition {
    HypercallDisposition::Answered(HypercallAnswer::lv1_status(r3, [0; 7]))
}

fn mac_answer(mac: u64) -> HypercallDisposition {
    HypercallDisposition::Answered(HypercallAnswer::lv1_status(0, [mac, 0, 0, 0, 0, 0, 0]))
}

fn refused(request: &HypercallRequest, what: &'static str) -> HypercallDisposition {
    HypercallDisposition::Refused(HypercallRefusal::UnmodelledInput {
        number: NET_CONTROL,
        unit: request.unit,
        pc: request.pc,
        what,
    })
}

/// [OFW-4.93 lv1 a:0x244F9C-0x244FA4] puts +16 of the device in r4 and
/// returns 0.
#[test]
fn the_kernel_call_gets_r3_0_the_synthetic_mac_address_in_r4_and_no_change() {
    assert_eq!(
        control(&kernel_call(), &open()),
        mac_answer(SYNTHETIC_CONSOLE_IDENTITY.mac_address)
    );
}

#[test]
fn request_1_gives_the_mac_address_of_the_identity_that_the_hypervisor_serves() {
    let other = ConsoleIdentity {
        mac_address: OTHER_MAC,
        ..SYNTHETIC_CONSOLE_IDENTITY
    };
    assert_ne!(other.mac_address, SYNTHETIC_CONSOLE_IDENTITY.mac_address);
    assert_eq!(
        net_control(
            &kernel_call(),
            Some("4.93"),
            &open(),
            &other,
            CONSOLE_VLAN_FIELDS
        ),
        mac_answer(OTHER_MAC)
    );
}

/// The request switch's path for request 1 reads no input past r5
/// [OFW-4.93 lv1 a:0x244F9C-0x244FA8].
#[test]
fn request_1_ignores_r6_to_r8() {
    let inputs = [[1, 0, 0], [0, 1, 0], [0, 0, 1], [u64::MAX; 3]];
    assert_eq!(inputs.len(), 4);
    for r6_to_r8 in inputs {
        assert_eq!(
            control(&request(1, 0, 1, r6_to_r8), &open()),
            mac_answer(SYNTHETIC_CONSOLE_IDENTITY.mac_address),
            "{r6_to_r8:x?}"
        );
    }
}

/// [OFW-4.93 lv1 a:0x307DD4-0x307DE8] checks the bus and the device
/// before the state word at [OFW-4.93 lv1 a:0x307E2C].
#[test]
fn each_failed_device_check_gets_its_marked_status_before_the_open_check() {
    let cases = [
        (5, 0, NET_CONTROL_BUS_RANGE_R3),
        ((1 << 32) | 1, 0, NET_CONTROL_BUS_RANGE_R3),
        (0, 0, NET_CONTROL_NO_BUS_R3),
        (2, 0, NET_CONTROL_NO_BUS_R3),
        (4, 0, NET_CONTROL_NO_BUS_R3),
        (1, 1, NET_CONTROL_NO_DEVICE_R3),
        (1, 1 << 32, NET_CONTROL_NO_DEVICE_R3),
    ];
    assert_eq!(cases.len(), 7);
    for devices in [DeviceBusState::default(), open()] {
        for (bus, device, r3) in cases {
            assert_eq!(
                control(&request(bus, device, 1, [0; 3]), &devices),
                answered(r3),
                "bus {bus:#x} device {device:#x}"
            );
        }
    }
}

/// [OFW-4.93 lv1 a:0x307F40-0x307F48] and
/// [OFW-4.93 lv1 a:0x307F84-0x307F98] both fail these codes.
#[test]
fn a_code_outside_both_request_ranges_gets_r5_marked() {
    let codes = [0, NET_CONTROL_PRIVILEGED_REQUEST_MAX + 1, u64::MAX];
    assert_eq!(codes.len(), 3);
    for code in codes {
        assert_eq!(
            control(&request(1, 0, code, [0; 3]), &open()),
            answered(NET_CONTROL_REQUEST_RANGE_R3),
            "code {code:#x}"
        );
    }
}

#[test]
fn a_code_that_only_the_privilege_bit_passes_is_a_refusal() {
    let codes = [
        NET_CONTROL_REQUEST_MAX + 1,
        0x8000_0000_0000_0001,
        NET_CONTROL_PRIVILEGED_REQUEST_MAX,
    ];
    assert_eq!(codes.len(), 3);
    for code in codes {
        let call = request(1, 0, code, [0; 3]);
        assert_eq!(
            control(&call, &open()),
            refused(&call, PRIVILEGED_REQUEST.what),
            "code {code:#x}"
        );
    }
}

#[test]
fn a_code_from_2_to_11_other_than_4_is_a_refusal() {
    let codes: Vec<u64> = (2..=NET_CONTROL_REQUEST_MAX)
        .filter(|&code| code != 4)
        .collect();
    assert_eq!(codes.len(), 9);
    for code in codes {
        let call = request(1, 0, code, [0; 3]);
        assert_eq!(
            control(&call, &open()),
            refused(&call, UNMODELLED_REQUEST.what),
            "code {code}"
        );
    }
}

#[test]
fn a_call_before_the_partition_opened_the_network_device_is_a_refusal() {
    let mut other = DeviceBusState::default();
    other.apply(DeviceBusChange::Opened(OpenDevice { bus: 1, device: 1 }));
    for devices in [DeviceBusState::default(), other] {
        let call = kernel_call();
        assert_eq!(
            control(&call, &devices),
            refused(&call, DEVICE_NOT_OPEN.what)
        );
    }
}

/// The state word check [OFW-4.93 lv1 a:0x307E2C-0x307E40] comes before
/// the request check [OFW-4.93 lv1 a:0x307F40-0x307F48]. So a closed
/// device gives the refusal for every request code.
#[test]
fn the_open_check_comes_before_the_request_check() {
    let codes = [
        0,
        2,
        NET_CONTROL_REQUEST_MAX + 1,
        NET_CONTROL_PRIVILEGED_REQUEST_MAX + 1,
    ];
    for code in codes {
        let call = request(1, 0, code, [0; 3]);
        assert_eq!(
            control(&call, &DeviceBusState::default()),
            refused(&call, DEVICE_NOT_OPEN.what),
            "code {code:#x}"
        );
    }
}

#[test]
fn a_call_at_a_firmware_whose_handler_is_not_read_is_a_refusal() {
    let call = kernel_call();
    for firmware in [None, Some("3.70"), Some("4.92")] {
        assert_eq!(
            net_control(
                &call,
                firmware,
                &open(),
                &SYNTHETIC_CONSOLE_IDENTITY,
                CONSOLE_VLAN_FIELDS
            ),
            refused(&call, UNREAD_FIRMWARE.what),
            "{firmware:?}"
        );
    }
}

#[test]
fn the_hypervisor_answers_194_after_the_open_without_a_state_change() {
    assert!(answered_numbers().contains(&NET_CONTROL));
    let mut hv = Hypervisor::new();
    hv.set_firmware("4.93");
    let open = HypercallRequest {
        number: OPEN_DEVICE,
        ..request(1, 0, 0, [0; 3])
    };
    let HypercallDisposition::Answered(opened) = hv.dispatch(&open) else {
        panic!("open_device refused");
    };
    assert_eq!(opened.r3, 0);
    let before = hv.clone();
    let HypercallDisposition::Answered(answer) = hv.dispatch(&kernel_call()) else {
        panic!("net_control refused");
    };
    assert_eq!(answer.r3, 0);
    assert_eq!(
        answer.outputs().map(|registers| registers[0]),
        Some(SYNTHETIC_CONSOLE_IDENTITY.mac_address)
    );
    assert_eq!(hv, before);
}
