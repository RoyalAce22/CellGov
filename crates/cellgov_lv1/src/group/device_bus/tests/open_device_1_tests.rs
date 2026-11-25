use super::*;
use crate::Hypervisor;
use cellgov_event::UnitId;
use cellgov_time::GuestTicks;

const BUS_1_DEVICE_0: OpenDevice = OpenDevice { bus: 1, device: 0 };
const BUS_1_DEVICE_1: OpenDevice = OpenDevice { bus: 1, device: 1 };

/// The call for device `device` of bus 1 from the 4.93 kernel's
/// `open_device` wrapper [OFW-4.93 lv2_kernel a:0x8000000000095718], with
/// `r5` as its third input.
fn request(device: u64, r5: u64) -> HypercallRequest {
    let mut inputs = [0; 8];
    inputs[0] = 1;
    inputs[1] = device;
    inputs[2] = r5;
    HypercallRequest {
        unit: UnitId::new(0),
        number: OPEN_DEVICE,
        inputs,
        pc: 0x8000_0000_0009_5718,
        lr: 0,
        thread: Some(0),
        problem_state: false,
        tick: GuestTicks::new(28_408),
    }
}

/// The kernel's call: bus 1, device 1 and r5 = 0
/// [OFW-4.93 lv2_kernel a:0x8000000000095704-0x8000000000095718].
fn kernel_call() -> HypercallRequest {
    request(1, 0)
}

fn opened(devices: &[OpenDevice]) -> DeviceBusState {
    let mut state = DeviceBusState::default();
    for device in devices {
        state.apply(DeviceBusChange::Opened(*device));
    }
    state
}

fn opens(device: OpenDevice) -> HypercallDisposition {
    let mut answer = HypercallAnswer::lv1_status(0, [0; 7]);
    answer.push_change(DeviceBusChange::Opened(device));
    HypercallDisposition::Answered(answer)
}

fn r3_of(disposition: &HypercallDisposition) -> Option<u64> {
    match disposition {
        HypercallDisposition::Answered(answer) => Some(answer.r3),
        HypercallDisposition::Refused(_) => None,
    }
}

/// [OFW-4.93 lv1 a:0x2280B0-0x2280BC] sets the open flag and returns 0;
/// [OFW-4.93 lv1 a:0x23DD14-0x23DD28] returns 0 from the controller's
/// open; [OFW-4.93 lv1 a:0x2377EC] clears the high word.
#[test]
fn the_kernel_open_of_bus_1_device_1_gets_r3_0_with_r4_to_r10_cleared_and_opens_it() {
    assert_eq!(
        open_device(&kernel_call(), Some("4.93"), &DeviceBusState::default()),
        opens(BUS_1_DEVICE_1)
    );
}

/// [OFW-4.93 lv1 a:0x2280A0-0x2280AC] returns -9 for a set open flag;
/// [OFW-4.93 lv1 a:0x23DC78-0x23DCBC] passes it on;
/// [OFW-4.93 lv1 a:0x2377EC] clears the high word.
#[test]
fn an_open_of_bus_1_device_1_that_is_open_gets_busy_and_no_change() {
    assert_eq!(
        open_device(&kernel_call(), Some("4.93"), &opened(&[BUS_1_DEVICE_1])),
        HypercallDisposition::Answered(HypercallAnswer::lv1_status(0x0000_0000_FFFF_FFF7, [0; 7]))
    );
}

/// Each device keeps its own open flag at +108
/// [OFW-4.93 lv1 a:0x2280A0-0x2280BC].
#[test]
fn an_open_of_device_0_or_device_1_does_not_depend_on_the_other_one() {
    assert_eq!(
        open_device(&kernel_call(), Some("4.93"), &opened(&[BUS_1_DEVICE_0])),
        opens(BUS_1_DEVICE_1)
    );
    assert_eq!(
        open_device(&request(0, 0), Some("4.93"), &opened(&[BUS_1_DEVICE_1])),
        opens(BUS_1_DEVICE_0)
    );
}

/// [OFW-4.93 lv1 a:0x2377DC-0x2377F0] passes only the partition to the
/// controller's open.
#[test]
fn the_device_1_answer_ignores_r5() {
    let empty = DeviceBusState::default();
    let open = opened(&[BUS_1_DEVICE_1]);
    let expected = (
        open_device(&kernel_call(), Some("4.93"), &empty),
        open_device(&kernel_call(), Some("4.93"), &open),
    );
    for r5 in [1, 0x8000_0000_0000_0000, u64::MAX] {
        let call = request(1, r5);
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

/// The 4.93 kernel opens device 0 of bus 1 and then device 1
/// [OFW-4.93 lv2_kernel a:0x80000000000936cc-0x80000000000936ec]
/// [OFW-4.93 lv2_kernel a:0x8000000000093634-0x8000000000093664].
#[test]
fn the_hypervisor_opens_device_0_then_device_1_as_the_kernel_does() {
    let mut hv = Hypervisor::new();
    hv.set_firmware("4.93");
    assert_eq!(r3_of(&hv.dispatch(&request(0, 0))), Some(0));
    let after_device_0 = hv.sync_term();
    assert_eq!(r3_of(&hv.dispatch(&kernel_call())), Some(0));
    let open: Vec<_> = hv.groups().device_bus().open_devices().collect();
    assert_eq!(open, [BUS_1_DEVICE_0, BUS_1_DEVICE_1]);
    assert_ne!(hv.sync_term(), after_device_0);
}
