use super::*;
use crate::answer::HypercallRefusal;
use crate::group::device_bus::DeviceBusChange;
use crate::Hypervisor;
use cellgov_event::UnitId;
use cellgov_ps3_abi::lv1::call::OPEN_DEVICE;
use cellgov_time::GuestTicks;

/// The `sc` of the 4.93 kernel's request 4
/// [OFW-4.93 lv2_kernel a:0x80000000000A2868].
const KERNEL_SC: u64 = 0x8000_0000_000A_2868;

/// Request 4 for device 0 of bus 1 with `selector` in r6 and `r7_r8` in
/// r7 and r8.
fn request(selector: u64, r7_r8: [u64; 2]) -> HypercallRequest {
    HypercallRequest {
        unit: UnitId::new(0),
        number: NET_CONTROL,
        inputs: [1, 0, 4, selector, r7_r8[0], r7_r8[1], 0, 0],
        pc: KERNEL_SC,
        lr: 0,
        thread: Some(0),
        problem_state: false,
        tick: GuestTicks::new(33_289),
    }
}

/// The kernel's call for `selector`: 0 in r7 and r8
/// [OFW-4.93 lv2_kernel a:0x80000000000A2844-0x80000000000A2864].
fn kernel_call(selector: u64) -> HypercallRequest {
    request(selector, [0; 2])
}

fn open() -> DeviceBusState {
    let mut devices = DeviceBusState::default();
    devices.apply(DeviceBusChange::Opened(NETWORK_DEVICE));
    devices
}

fn control(r: &HypercallRequest) -> HypercallDisposition {
    net_control(r, Some("4.93"), &open(), &SYNTHETIC_CONSOLE_IDENTITY)
}

fn id_answer(id: u64) -> HypercallDisposition {
    HypercallDisposition::Answered(HypercallAnswer::lv1_status(0, [id, 0, 0, 0, 0, 0, 0]))
}

/// The kernel's selectors before selector 5, in its order, with the id
/// of each [OFW-4.93 lv2_kernel a:0x80000000000A384C-0x80000000000A3A24]
/// [OFW-4.93 lv1 a:0x244E88-0x244F30].
const KERNEL_SELECTORS: [(u64, u64); 10] = [
    (1, 292),
    (16, 288),
    (17, 289),
    (18, 290),
    (19, 261),
    (20, 256),
    (1, 292),
    (2, 294),
    (3, 295),
    (4, 291),
];

#[test]
fn each_kernel_selector_before_5_gets_r3_0_and_its_id_in_r4() {
    assert_eq!(KERNEL_SELECTORS.len(), 10);
    for (selector, id) in KERNEL_SELECTORS {
        assert_eq!(
            control(&kernel_call(selector)),
            id_answer(id),
            "selector {selector}"
        );
    }
}

#[test]
fn the_kernel_selector_5_is_a_refusal() {
    let call = kernel_call(5);
    assert_eq!(
        control(&call),
        HypercallDisposition::Refused(HypercallRefusal::UnmodelledInput {
            number: NET_CONTROL,
            unit: call.unit,
            pc: call.pc,
            what: FLAGGED_VLAN_SELECTOR.what,
        })
    );
}

/// [OFW-4.93 lv1 a:0x244B24-0x244B54] stores no output, and
/// [OFW-4.93 lv1 a:0x307F7C] keeps the low word of -17. The handler
/// keeps all 64 bits of r6 [OFW-4.93 lv1 a:0x307D6C] and the range check
/// compares all 64 [OFW-4.93 lv1 a:0x244DF0-0x244DF4], so a selector
/// whose low word is 1 fails.
#[test]
fn a_selector_without_an_id_gets_minus_17_zero_extended_and_r4_cleared() {
    let selectors = [0, 6, 15, 21, 1 << 32, (1 << 32) | 1, u64::MAX];
    assert_eq!(selectors.len(), 7);
    for selector in selectors {
        assert_eq!(
            control(&kernel_call(selector)),
            HypercallDisposition::Answered(HypercallAnswer::lv1_status(
                0x0000_0000_FFFF_FFEF,
                [0; 7]
            )),
            "selector {selector:#x}"
        );
    }
}

/// The case at [OFW-4.93 lv1 a:0x244DD8-0x244F30] reads r6 and no
/// other input.
#[test]
fn request_4_ignores_r7_and_r8() {
    let inputs = [[1, 0], [0, 1], [u64::MAX; 2]];
    assert_eq!(inputs.len(), 3);
    for r7_r8 in inputs {
        assert_eq!(control(&request(16, r7_r8)), id_answer(288), "{r7_r8:x?}");
    }
}

#[test]
fn the_hypervisor_answers_request_4_after_the_open_without_a_state_change() {
    let mut hv = Hypervisor::new();
    hv.set_firmware("4.93");
    let open = HypercallRequest {
        number: OPEN_DEVICE,
        inputs: [1, 0, 0, 0, 0, 0, 0, 0],
        ..kernel_call(0)
    };
    let HypercallDisposition::Answered(opened) = hv.dispatch(&open) else {
        panic!("open_device refused");
    };
    assert_eq!(opened.r3, 0);
    let before = hv.clone();
    let HypercallDisposition::Answered(answer) = hv.dispatch(&kernel_call(1)) else {
        panic!("net_control refused");
    };
    assert_eq!(answer.r3, 0);
    assert_eq!(answer.outputs().map(|registers| registers[0]), Some(292));
    assert_eq!(hv, before);
}
