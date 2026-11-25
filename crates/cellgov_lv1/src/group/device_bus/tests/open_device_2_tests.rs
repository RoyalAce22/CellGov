use super::*;
use crate::Hypervisor;
use cellgov_event::UnitId;
use cellgov_time::GuestTicks;

const BUS_1_DEVICE_1: OpenDevice = OpenDevice { bus: 1, device: 1 };
const BUS_1_DEVICE_2: OpenDevice = OpenDevice { bus: 1, device: 2 };

/// The call for device `device` of bus 1 from the 4.93 kernel's
/// `open_device` wrapper [OFW-4.93 lv2_kernel a:0x8000000000095718], with
/// r5 = 0.
fn request(device: u64) -> HypercallRequest {
    HypercallRequest {
        unit: UnitId::new(0),
        number: OPEN_DEVICE,
        inputs: [1, device, 0, 0, 0, 0, 0, 0],
        pc: 0x8000_0000_0009_5718,
        lr: 0,
        thread: Some(0),
        problem_state: false,
        tick: GuestTicks::new(28_473),
    }
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

/// [OFW-4.93 lv1 a:0x237760-0x23777C] opens device 2 through the second
/// controller, which returns 0 after the device's open entry returned 0
/// [OFW-4.93 lv1 a:0x2280B0-0x2280BC] [OFW-4.93 lv1 a:0x23DD14-0x23DD28].
#[test]
fn the_kernel_open_of_bus_1_device_2_gets_r3_0_with_r4_to_r10_cleared_and_opens_it() {
    assert_eq!(
        open_device(&request(2), Some("4.93"), &DeviceBusState::default()),
        opens(BUS_1_DEVICE_2)
    );
}

/// [OFW-4.93 lv1 a:0x2280A0-0x2280AC] returns -9 for a set open flag;
/// [OFW-4.93 lv1 a:0x237778] clears the high word.
#[test]
fn an_open_of_bus_1_device_2_that_is_open_gets_busy_and_no_change() {
    assert_eq!(
        open_device(&request(2), Some("4.93"), &opened(&[BUS_1_DEVICE_2])),
        HypercallDisposition::Answered(HypercallAnswer::lv1_status(0x0000_0000_FFFF_FFF7, [0; 7]))
    );
}

/// Devices 1 and 2 have controllers and open flags of their own
/// [OFW-4.93 lv1 a:0x23DA60-0x23DB54].
#[test]
fn an_open_of_device_1_or_device_2_does_not_depend_on_the_other_one() {
    assert_eq!(
        open_device(&request(2), Some("4.93"), &opened(&[BUS_1_DEVICE_1])),
        opens(BUS_1_DEVICE_2)
    );
    assert_eq!(
        open_device(&request(1), Some("4.93"), &opened(&[BUS_1_DEVICE_2])),
        opens(BUS_1_DEVICE_1)
    );
}

#[test]
fn the_hypervisor_opens_devices_0_1_and_2_as_the_kernel_does() {
    let mut hv = Hypervisor::new();
    hv.set_firmware("4.93");
    for device in [0, 1, 2] {
        let HypercallDisposition::Answered(answer) = hv.dispatch(&request(device)) else {
            panic!("the open of device {device} is refused");
        };
        assert_eq!(answer.r3, 0, "device {device}");
    }
    let open: Vec<_> = hv.groups().device_bus().open_devices().collect();
    assert_eq!(
        open,
        [
            OpenDevice { bus: 1, device: 0 },
            BUS_1_DEVICE_1,
            BUS_1_DEVICE_2
        ]
    );
}
