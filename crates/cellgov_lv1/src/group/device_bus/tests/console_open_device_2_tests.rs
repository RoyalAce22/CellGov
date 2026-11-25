//! The hypervisor leaves device 2 of bus 1 open after the 4.93 kernel's
//! open, as lv1 does on the reference console, read from the committed
//! capture `tests/micro/lv1_device_bus_open`.

use super::console_device_1_tests::{controller_capture, ControllerPath, CONTROLLER_OPENED};
use super::console_tests::kernel_request;
use super::*;
use crate::Hypervisor;
use cellgov_ps3_abi::lv1::call::GET_LOGICAL_PARTITION_ID;

/// Device 2 is in slot 4 [OFW-4.93 lv1 a:0x235DB0-0x235DB8], and its
/// controller is in slot 1 of the array [OFW-4.93 lv1 a:0x237768-0x23776C].
const DEVICE_2: ControllerPath = ControllerPath {
    files: "device2",
    slot: 4,
    array_slot: 1,
};

/// The kernel opens device 2 of bus 1 with r5 = 0, after devices 0 and 1
/// [OFW-4.93 lv2_kernel a:0x8000000000093484-0x8000000000093490]
/// [OFW-4.93 lv2_kernel a:0x8000000000095704-0x8000000000095718].
#[test]
fn the_kernel_open_leaves_bus_1_device_2_open_for_its_partition_as_lv1_does() {
    let (flag, partition, state) = controller_capture(&DEVICE_2);
    let mut hv = Hypervisor::new();
    hv.set_firmware("4.93");
    for device in [0, 1] {
        let earlier = hv.dispatch(&kernel_request(OPEN_DEVICE, [1, device, 0, 0, 0, 0, 0, 0]));
        assert!(
            matches!(earlier, HypercallDisposition::Answered(_)),
            "device {device}"
        );
    }
    let HypercallDisposition::Answered(answer) =
        hv.dispatch(&kernel_request(OPEN_DEVICE, [1, 2, 0, 0, 0, 0, 0, 0]))
    else {
        panic!("the kernel's open of device 2 is refused");
    };
    assert_eq!(answer.r3, 0, "r3");
    assert_eq!(state, CONTROLLER_OPENED, "the controller's state word");
    let open = hv
        .groups()
        .device_bus()
        .is_open(OpenDevice { bus: 1, device: 2 });
    assert_eq!(u64::from(open), flag, "the open flag");
    let HypercallDisposition::Answered(id) =
        hv.dispatch(&kernel_request(GET_LOGICAL_PARTITION_ID, [0; 8]))
    else {
        panic!("get_logical_partition_id is refused");
    };
    assert_eq!(
        id.outputs().map(|outputs| outputs[0]),
        Some(partition),
        "the partition that keeps the device open"
    );
}
