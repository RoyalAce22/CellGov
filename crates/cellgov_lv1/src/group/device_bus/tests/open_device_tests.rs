use super::*;
use crate::answer::HypercallRefusal;
use crate::{answered_numbers, Hypervisor};
use cellgov_event::UnitId;
use cellgov_ps3_abi::lv1::device_bus::{OPEN_DEVICE_BUS_RANGE_R3, OPEN_DEVICE_NO_BUS_R3};
use cellgov_time::GuestTicks;

/// The `sc` of the 4.93 kernel's `open_device` wrapper
/// [OFW-4.93 lv2_kernel a:0x8000000000095718].
const KERNEL_SC: u64 = 0x8000_0000_0009_5718;

/// The call for device `device` of bus `bus`, with `r5` as its third
/// input.
fn request(bus: u64, device: u64, r5: u64) -> HypercallRequest {
    let mut inputs = [0; 8];
    inputs[0] = bus;
    inputs[1] = device;
    inputs[2] = r5;
    HypercallRequest {
        unit: UnitId::new(0),
        number: OPEN_DEVICE,
        inputs,
        pc: KERNEL_SC,
        lr: 0,
        thread: Some(0),
        problem_state: false,
        tick: GuestTicks::new(28_167),
    }
}

/// The kernel's call: bus 1, device 0 and r5 = 0
/// [OFW-4.93 lv2_kernel a:0x8000000000095704-0x8000000000095718].
fn kernel_call() -> HypercallRequest {
    request(1, 0, 0)
}

const BUS_1_DEVICE_0: OpenDevice = OpenDevice { bus: 1, device: 0 };

fn opened(device: OpenDevice) -> DeviceBusState {
    let mut state = DeviceBusState::default();
    state.apply(DeviceBusChange::Opened(device));
    state
}

fn answered(r3: u64) -> HypercallDisposition {
    HypercallDisposition::Answered(HypercallAnswer::lv1_status(r3, [0; 7]))
}

fn refused(request: &HypercallRequest, what: &'static str) -> HypercallDisposition {
    HypercallDisposition::Refused(HypercallRefusal::UnmodelledInput {
        number: OPEN_DEVICE,
        unit: request.unit,
        pc: request.pc,
        what,
    })
}

fn r3_of(disposition: &HypercallDisposition) -> Option<u64> {
    match disposition {
        HypercallDisposition::Answered(answer) => Some(answer.r3),
        _ => None,
    }
}

/// [OFW-4.93 lv1 a:0x2280B0-0x2280BC] sets the open flag and returns 0;
/// [OFW-4.93 lv1 a:0x2377A8] returns 0 after the hook.
#[test]
fn the_kernel_call_gets_r3_0_with_r4_to_r10_cleared_and_opens_bus_1_device_0() {
    let mut expected = HypercallAnswer::lv1_status(0, [0; 7]);
    expected.push_change(DeviceBusChange::Opened(BUS_1_DEVICE_0));
    assert_eq!(
        open_device(&kernel_call(), Some("4.93"), &DeviceBusState::default()),
        HypercallDisposition::Answered(expected)
    );
}

/// [OFW-4.93 lv1 a:0x2280A0-0x2280AC] returns -9 for a set open flag;
/// [OFW-4.93 lv1 a:0x2376C0-0x2376C8] clears the high word.
#[test]
fn an_open_of_bus_1_device_0_that_is_open_gets_busy_and_no_change() {
    assert_eq!(
        open_device(&kernel_call(), Some("4.93"), &opened(BUS_1_DEVICE_0)),
        answered(0x0000_0000_FFFF_FFF7)
    );
}

/// [OFW-4.93 lv1 a:0x228078-0x2280D4] reads no r5.
#[test]
fn the_answer_ignores_r5() {
    let empty = DeviceBusState::default();
    let open = opened(BUS_1_DEVICE_0);
    let mut opens = HypercallAnswer::lv1_status(0, [0; 7]);
    opens.push_change(DeviceBusChange::Opened(BUS_1_DEVICE_0));
    let expected = (
        HypercallDisposition::Answered(opens),
        answered(0x0000_0000_FFFF_FFF7),
    );
    for r5 in [1, 0x8000_0000_0000_0000, u64::MAX] {
        let call = request(1, 0, r5);
        assert_eq!(
            (
                open_device(&call, Some("4.93"), &empty),
                open_device(&call, Some("4.93"), &open),
            ),
            expected,
            "r5 0x{r5:X}"
        );
    }
}

/// [OFW-4.93 lv1 a:0x305AC8-0x305ACC] [OFW-4.93 lv1 a:0x305B78-0x305B84]
/// refuse a bus id above 4; [OFW-4.93 lv1 a:0x2375C0-0x2375E8]
/// [OFW-4.93 lv1 a:0x237714-0x237754] refuse bus id 0.
#[test]
fn each_failed_bus_check_gets_its_marked_status_for_any_device() {
    let cases = [
        (0, OPEN_DEVICE_NO_BUS_R3, 0x0000_0001_FFFF_FFFA),
        (5, OPEN_DEVICE_BUS_RANGE_R3, 0x0000_0001_FFFF_FFEF),
        (
            0x0000_0001_0000_0001,
            OPEN_DEVICE_BUS_RANGE_R3,
            0x0000_0001_FFFF_FFEF,
        ),
        (u64::MAX, OPEN_DEVICE_BUS_RANGE_R3, 0x0000_0001_FFFF_FFEF),
    ];
    assert_eq!(cases.len(), 4);
    for (bus, r3, value) in cases {
        assert_eq!(r3, value, "bus {bus}");
        for device in [0, 1, 7] {
            assert_eq!(
                open_device(
                    &request(bus, device, 0),
                    Some("4.93"),
                    &opened(BUS_1_DEVICE_0)
                ),
                answered(value),
                "bus {bus} device {device}"
            );
        }
    }
}

#[test]
fn an_open_of_another_device_on_a_bus_that_passes_the_checks_is_a_refusal() {
    let what = "an open of a device other than devices 0, 1 and 2 of bus 1, which lv1 finds \
                through state that CellGov does not model";
    let cases = [
        (1, 3),
        (1, 4),
        (2, 0),
        (3, 0),
        (4, 0),
        (4, 9),
        (1, 0x0000_0001_0000_0000),
    ];
    assert_eq!(cases.len(), 7);
    for (bus, device) in cases {
        let call = request(bus, device, 0);
        assert_eq!(
            open_device(&call, Some("4.93"), &DeviceBusState::default()),
            refused(&call, what),
            "bus {bus} device {device}"
        );
    }
}

#[test]
fn an_open_at_a_firmware_whose_handler_is_not_read_is_a_refusal() {
    let what = "an open_device at a firmware whose lv1 handler is not read";
    for firmware in [None, Some("4.92"), Some("3.70")] {
        assert_eq!(
            open_device(&kernel_call(), firmware, &DeviceBusState::default()),
            refused(&kernel_call(), what),
            "{firmware:?}"
        );
    }
}

#[test]
fn the_hypervisor_answers_170_then_gets_busy_on_a_second_open() {
    assert!(answered_numbers().contains(&OPEN_DEVICE));
    let mut hv = Hypervisor::new();
    hv.set_firmware("4.93");
    let empty = hv.sync_term();
    assert_eq!(r3_of(&hv.dispatch(&kernel_call())), Some(0));
    let open: Vec<_> = hv.groups().device_bus().open_devices().collect();
    assert_eq!(open, [BUS_1_DEVICE_0]);
    assert_ne!(hv.sync_term(), empty);
    let after_first = hv.clone();
    assert_eq!(
        r3_of(&hv.dispatch(&kernel_call())),
        Some(0x0000_0000_FFFF_FFF7)
    );
    assert_eq!(hv, after_first);
}

#[test]
fn a_failed_bus_check_leaves_the_hypervisor_unchanged() {
    let mut hv = Hypervisor::new();
    hv.set_firmware("4.93");
    let before = hv.clone();
    assert_eq!(
        r3_of(&hv.dispatch(&request(0, 0, 0))),
        Some(0x0000_0001_FFFF_FFFA)
    );
    assert_eq!(hv, before);
}
