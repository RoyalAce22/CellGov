use super::*;
use crate::answer::HypercallRefusal;
use crate::group::device_bus::{DeviceBusChange, OpenDevice};
use crate::group::network::indicator::NETWORK_DEVICE;
use crate::{answered_numbers, Hypervisor};
use cellgov_event::UnitId;
use cellgov_ps3_abi::lv1::call::OPEN_DEVICE;
use cellgov_ps3_abi::lv1::network::{
    INTERRUPT_MASK_BUS_RANGE_R3, INTERRUPT_MASK_NO_BUS_R3, INTERRUPT_MASK_NO_DEVICE_R3,
};
use cellgov_time::GuestTicks;

/// The `sc` of the 4.93 kernel's first call on the boot path
/// [OFW-4.93 lv2_kernel a:0x80000000000A568C].
const KERNEL_SC: u64 = 0x8000_0000_000A_568C;

/// The mask of that call for a record whose doubleword at +64 is 0
/// [OFW-4.93 lv2_kernel a:0x80000000000A5668-0x80000000000A5680].
const KERNEL_MASK: u64 = 0x1002_0000_0000_6000;

/// The call for device `device` of bus `bus`, with `mask` in r5 and `r6`.
fn request(bus: u64, device: u64, mask: u64, r6: u64) -> HypercallRequest {
    let mut registers = [0; 8];
    registers[..4].copy_from_slice(&[bus, device, mask, r6]);
    HypercallRequest {
        unit: UnitId::new(0),
        number: NET_SET_INTERRUPT_MASK,
        inputs: registers,
        pc: KERNEL_SC,
        lr: 0,
        thread: Some(0),
        problem_state: false,
        tick: GuestTicks::new(33_285),
    }
}

/// The kernel's call: bus 1, device 0, the kernel's mask and 0 in r6
/// [OFW-4.93 lv2_kernel a:0x80000000000A565C-0x80000000000A5688].
fn kernel_call() -> HypercallRequest {
    request(1, 0, KERNEL_MASK, 0)
}

/// The device bus state with the network device open.
fn open() -> DeviceBusState {
    let mut devices = DeviceBusState::default();
    devices.apply(DeviceBusChange::Opened(NETWORK_DEVICE));
    devices
}

fn set_mask(r: &HypercallRequest, devices: &DeviceBusState) -> HypercallDisposition {
    set_interrupt_mask(r, Some("4.93"), devices, &NetworkState::default())
}

fn answered(r3: u64) -> HypercallDisposition {
    HypercallDisposition::Answered(HypercallAnswer::lv1_status(r3, [0; 7]))
}

fn mask_set(mask: u64) -> HypercallDisposition {
    let mut answer = HypercallAnswer::lv1_status(0, [0; 7]);
    answer.push_change(NetworkChange::InterruptMaskSet(mask));
    HypercallDisposition::Answered(answer)
}

fn refused(request: &HypercallRequest, what: &'static str) -> HypercallDisposition {
    HypercallDisposition::Refused(HypercallRefusal::UnmodelledInput {
        number: NET_SET_INTERRUPT_MASK,
        unit: request.unit,
        pc: request.pc,
        what,
    })
}

/// [OFW-4.93 lv1 a:0x3081C8-0x3081CC] stores r5 at +32 of the device and
/// returns 0.
#[test]
fn the_kernel_call_gets_r3_0_cleared_r4_to_r10_and_sets_its_mask() {
    assert_eq!(set_mask(&kernel_call(), &open()), mask_set(KERNEL_MASK));
}

/// The handler checks no bit of r5 [OFW-4.93 lv1 a:0x307FE0-0x308200].
/// The masks are each kernel call's bits over a record doubleword of 0
/// [OFW-4.93 lv2_kernel a:0x80000000000A3160-0x80000000000A37A0]
/// [OFW-4.93 lv2_kernel a:0x80000000000A49B8-0x80000000000A4A00]
/// [OFW-4.93 lv2_kernel a:0x80000000000A7FCC-0x80000000000A7FEC].
#[test]
fn each_mask_is_set_as_given() {
    let masks = [
        0,
        0x0C51_A00C,
        0x2C51_A00C,
        0x0180_2000_0000_0000,
        KERNEL_MASK,
        0xC000_0000,
        u64::MAX,
    ];
    assert_eq!(masks.len(), 7);
    for mask in masks {
        assert_eq!(
            set_mask(&request(1, 0, mask, 0), &open()),
            mask_set(mask),
            "mask {mask:#x}"
        );
    }
}

/// [OFW-4.93 lv1 a:0x308070-0x308084] checks the bus and the device
/// before the state word at [OFW-4.93 lv1 a:0x3080C8].
#[test]
fn each_failed_device_check_gets_its_marked_status_before_the_open_check() {
    let cases = [
        (5, 0, INTERRUPT_MASK_BUS_RANGE_R3),
        ((1 << 32) | 1, 0, INTERRUPT_MASK_BUS_RANGE_R3),
        (0, 0, INTERRUPT_MASK_NO_BUS_R3),
        (2, 0, INTERRUPT_MASK_NO_BUS_R3),
        (4, 0, INTERRUPT_MASK_NO_BUS_R3),
        (1, 1, INTERRUPT_MASK_NO_DEVICE_R3),
        (1, 1 << 32, INTERRUPT_MASK_NO_DEVICE_R3),
    ];
    assert_eq!(cases.len(), 7);
    for devices in [DeviceBusState::default(), open()] {
        for (bus, device, r3) in cases {
            assert_eq!(
                set_mask(&request(bus, device, KERNEL_MASK, 1), &devices),
                answered(r3),
                "bus {bus:#x} device {device:#x}"
            );
        }
    }
}

/// [OFW-4.93 lv1 a:0x308198-0x3081AC] refuses an r6 other than 0 before
/// the store at [OFW-4.93 lv1 a:0x3081CC].
#[test]
fn an_r6_other_than_0_gets_r6_marked_and_sets_no_mask() {
    let values = [1, 1 << 32, u64::MAX];
    assert_eq!(values.len(), 3);
    for r6 in values {
        assert_eq!(
            set_mask(&request(1, 0, KERNEL_MASK, r6), &open()),
            answered(INTERRUPT_MASK_R6_R3),
            "r6 {r6:#x}"
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
            set_mask(&call, &devices),
            refused(&call, DEVICE_NOT_OPEN.what)
        );
    }
}

/// The state word check [OFW-4.93 lv1 a:0x3080C8-0x3080D0] comes before
/// the r6 check [OFW-4.93 lv1 a:0x308198].
#[test]
fn the_open_check_comes_before_the_r6_check() {
    let call = request(1, 0, KERNEL_MASK, 1);
    assert_eq!(
        set_mask(&call, &DeviceBusState::default()),
        refused(&call, DEVICE_NOT_OPEN.what)
    );
}

#[test]
fn a_call_at_a_firmware_whose_handler_is_not_read_is_a_refusal_before_any_check() {
    let calls = [kernel_call(), request(5, 0, KERNEL_MASK, 1)];
    for firmware in [None, Some("3.70"), Some("4.92")] {
        for call in &calls {
            assert_eq!(
                set_interrupt_mask(call, firmware, &open(), &NetworkState::default()),
                refused(call, UNREAD_FIRMWARE.what),
                "{firmware:?} bus {:#x}",
                call.r(3)
            );
        }
        let call = kernel_call();
        assert_eq!(
            set_interrupt_mask(
                &call,
                firmware,
                &DeviceBusState::default(),
                &NetworkState::default()
            ),
            refused(&call, UNREAD_FIRMWARE.what),
            "{firmware:?} before the open"
        );
    }
}

#[test]
fn the_hypervisor_answers_193_after_the_open_and_keeps_the_last_mask() {
    assert!(answered_numbers().contains(&NET_SET_INTERRUPT_MASK));
    let mut hv = Hypervisor::new();
    hv.set_firmware("4.93");
    let open = HypercallRequest {
        number: OPEN_DEVICE,
        ..request(1, 0, 0, 0)
    };
    let HypercallDisposition::Answered(opened) = hv.dispatch(&open) else {
        panic!("open_device refused");
    };
    assert_eq!(opened.r3, 0);
    assert_eq!(hv.groups().network().interrupt_mask(), None);
    let before = hv.sync_term();
    for (mask, step) in [(KERNEL_MASK, "first"), (0, "second")] {
        let HypercallDisposition::Answered(answer) = hv.dispatch(&request(1, 0, mask, 0)) else {
            panic!("the {step} net_set_interrupt_mask refused");
        };
        assert_eq!(answer.r3, 0, "{step}");
        assert_eq!(answer.outputs(), Some([0; 7]), "{step}");
        assert_eq!(hv.groups().network().interrupt_mask(), Some(mask), "{step}");
        assert_ne!(hv.sync_term(), before, "{step}");
    }
}
