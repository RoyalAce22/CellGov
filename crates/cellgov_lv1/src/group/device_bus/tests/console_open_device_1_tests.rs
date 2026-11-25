//! The hypervisor leaves device 1 of bus 1 open after the 4.93 kernel's
//! open, as lv1 does on the reference console, read from the committed
//! capture `tests/micro/lv1_device_bus_open`.

use super::console_tests::{
    captured, kernel_request, one, DEVICE_METHOD_TABLE, OPEN_FLAG, PARTITION,
};
use super::*;
use crate::Hypervisor;
use cellgov_ps3_abi::lv1::call::GET_LOGICAL_PARTITION_ID;

/// The address of the controller array, which the getter's TOC slot
/// holds [OFW-4.93 lv1 a:0x353C88-0x353C8C].
const CONTROLLER_ARRAY: u64 = 0x36_82F8;

/// The controller's device, slot index and state word
/// [OFW-4.93 lv1 a:0x3AB040-0x3AB068] [OFW-4.93 lv1 a:0x23DAC4]
/// [OFW-4.93 lv1 a:0x23DD14-0x23DD28].
const CONTROLLER_DEVICE: u64 = 24;
const CONTROLLER_SLOT: u64 = 16;
const CONTROLLER_STATE: u64 = 240;

/// The state word that the controller's open stores after the device's
/// entry returned 0 [OFW-4.93 lv1 a:0x23DD20-0x23DD24].
pub(super) const CONTROLLER_OPENED: u64 = 2;

/// Where lv1 keeps a device of bus 1 that it opens through a controller.
pub(super) struct ControllerPath {
    /// The prefix of the device's capture files, such as `device1`.
    pub(super) files: &'static str,
    /// The system bus object slot that holds the device, from 0. Slot n
    /// is the doubleword at object + 32 + 8n
    /// [OFW-4.93 lv1 a:0x22B838-0x22B844].
    pub(super) slot: u64,
    /// The controller's slot in the controller array, from 0.
    pub(super) array_slot: u64,
}

/// The captured open flag, partition and controller state word of the
/// device at `path`, after a check that the capture read them from that
/// device and its controller.
pub(super) fn controller_capture(path: &ControllerPath) -> (u64, u64, u64) {
    let file = |name: &str| format!("{}_{name}.tsv", path.files);
    let walk = captured(&file("walk"), "doubleword");
    let [(_, bus_object), (slot, device), (device_at, method_table)] = walk[..] else {
        panic!("{} holds 3 rows: {walk:?}", file("walk"));
    };
    assert_eq!(slot, bus_object + 32 + 8 * path.slot, "the device's slot");
    assert_eq!(device_at, device, "the device's +0");
    assert_eq!(
        method_table, DEVICE_METHOD_TABLE,
        "the device's method table"
    );
    let controller = captured(&file("controller"), "doubleword");
    let [(array_at, controller), (held_at, held)] = controller[..] else {
        panic!("{} holds 2 rows: {controller:?}", file("controller"));
    };
    assert_eq!(
        array_at,
        CONTROLLER_ARRAY + 8 * path.array_slot,
        "the array's slot"
    );
    assert_eq!(
        held_at,
        controller + CONTROLLER_DEVICE,
        "the controller's +24"
    );
    assert_eq!(held, device, "the controller's device");
    let words = captured(&file("controller_state"), "word");
    let [(slot_at, slot_index), (state_at, state)] = words[..] else {
        panic!("{} holds 2 rows: {words:?}", file("controller_state"));
    };
    assert_eq!(
        slot_at,
        controller + CONTROLLER_SLOT,
        "the slot index's address"
    );
    assert_eq!(slot_index, path.slot, "the controller's slot index");
    assert_eq!(
        state_at,
        controller + CONTROLLER_STATE,
        "the state word's address"
    );
    let (flag_at, flag) = one(captured(&file("open_flag"), "byte"));
    let (partition_at, partition) = one(captured(&file("partition"), "doubleword"));
    assert_eq!(flag_at, device + OPEN_FLAG, "the open flag's address");
    assert_eq!(partition_at, device + PARTITION, "the partition's address");
    (flag, partition, state)
}

/// Device 1 is in slot 3 [OFW-4.93 lv1 a:0x235DA4-0x235DAC], and its
/// controller is in slot 0 of the array [OFW-4.93 lv1 a:0x2377DC-0x2377E0].
const DEVICE_1: ControllerPath = ControllerPath {
    files: "device1",
    slot: 3,
    array_slot: 0,
};

/// The kernel opens device 1 of bus 1 with r5 = 0, after device 0
/// [OFW-4.93 lv2_kernel a:0x8000000000093634-0x8000000000093664]
/// [OFW-4.93 lv2_kernel a:0x8000000000095704-0x8000000000095718].
#[test]
fn the_kernel_open_leaves_bus_1_device_1_open_for_its_partition_as_lv1_does() {
    let (flag, partition, state) = controller_capture(&DEVICE_1);
    let mut hv = Hypervisor::new();
    hv.set_firmware("4.93");
    let device_0 = hv.dispatch(&kernel_request(OPEN_DEVICE, [1, 0, 0, 0, 0, 0, 0, 0]));
    assert!(matches!(device_0, HypercallDisposition::Answered(_)));
    let HypercallDisposition::Answered(answer) =
        hv.dispatch(&kernel_request(OPEN_DEVICE, [1, 1, 0, 0, 0, 0, 0, 0]))
    else {
        panic!("the kernel's open of device 1 is refused");
    };
    assert_eq!(answer.r3, 0, "r3");
    assert_eq!(state, CONTROLLER_OPENED, "the controller's state word");
    let open = hv
        .groups()
        .device_bus()
        .is_open(OpenDevice { bus: 1, device: 1 });
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
