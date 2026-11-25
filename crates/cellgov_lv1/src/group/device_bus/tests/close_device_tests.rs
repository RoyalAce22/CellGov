use super::*;
use crate::answer::HypercallRefusal;
use crate::group::device_bus::{DmaMapping, DmaRegion, StorageBusOpen};
use cellgov_event::UnitId;
use cellgov_time::GuestTicks;

/// The `sc` of the 4.93 kernel's close of a device record's device
/// [OFW-4.93 lv2_kernel a:0x80000000000A357C].
const KERNEL_SC: u64 = 0x8000_0000_000A_357C;

const DEVICE_1: OpenDevice = OpenDevice { bus: 1, device: 1 };
const DEVICE_2: OpenDevice = OpenDevice { bus: 1, device: 2 };

/// The region that the console's lv1 gave the kernel's allocate call, from
/// the capture `lv1_device_dma_region`.
const REGION: DmaRegion = DmaRegion {
    bus: 1,
    device: 0,
    address: 0xC000_0000,
    size: 0x10_0000,
    page_shift: 20,
};

/// The mapping that the console's lv1 made for the kernel's map call, from
/// the capture `lv1_device_dma_map`.
const MAPPING: DmaMapping = DmaMapping {
    bus: 1,
    device: 0,
    bus_address: 0xC000_0000,
    lpar_address: 0x80_0000,
    length: 0x10_0000,
};

/// The call for device `device` of bus `bus`.
fn request(bus: u64, device: u64) -> HypercallRequest {
    let mut inputs = [0; 8];
    inputs[0] = bus;
    inputs[1] = device;
    HypercallRequest {
        unit: UnitId::new(0),
        number: CLOSE_DEVICE,
        inputs,
        pc: KERNEL_SC,
        lr: 0,
        thread: Some(0),
        problem_state: false,
        tick: GuestTicks::new(40_000),
    }
}

fn state_with(changes: &[DeviceBusChange]) -> DeviceBusState {
    let mut state = DeviceBusState::default();
    for change in changes {
        state.apply(change.clone());
    }
    state
}

fn close(state: &DeviceBusState, bus: u64, device: u64) -> HypercallDisposition {
    close_device(&request(bus, device), Some("4.93"), state)
}

fn answered(r3: u64) -> HypercallDisposition {
    HypercallDisposition::Answered(HypercallAnswer::lv1_status(r3, [0; 7]))
}

fn refused(request: &HypercallRequest, what: &'static str) -> HypercallDisposition {
    HypercallDisposition::Refused(HypercallRefusal::UnmodelledInput {
        number: CLOSE_DEVICE,
        unit: request.unit,
        pc: request.pc,
        what,
    })
}

/// [OFW-4.93 lv1 a:0x23E0F8-0x23E108] ends the teardown with 0;
/// [OFW-4.93 lv1 a:0x22968C-0x2296A0] returns 0 from the close entry;
/// [OFW-4.93 lv1 a:0x237564-0x237570] gives 0; the handler writes no
/// return slot [OFW-4.93 lv1 a:0x0C08].
#[test]
fn the_close_of_the_open_network_device_gets_0_closes_it_and_tears_it_down() {
    let state = state_with(&[DeviceBusChange::Opened(NETWORK_DEVICE)]);
    let mut expected = HypercallAnswer::lv1_status(0, [0; 7]);
    expected.push_change(DeviceBusChange::Closed(NETWORK_DEVICE));
    expected.push_change(NetworkChange::TornDown);
    assert_eq!(
        close(&state, 1, 0),
        HypercallDisposition::Answered(expected)
    );
}

/// [OFW-4.93 lv1 a:0x23757C-0x23758C] and
/// [OFW-4.93 lv1 a:0x237508-0x237518] call the controller's close, which
/// returns the close entry's 0 [OFW-4.93 lv1 a:0x23D8A4-0x23D8D4]. No
/// teardown of the network object runs on that path.
#[test]
fn the_close_of_open_device_1_or_2_gets_0_and_only_closes_it() {
    for device in [DEVICE_1, DEVICE_2] {
        let state = state_with(&[
            DeviceBusChange::Opened(NETWORK_DEVICE),
            DeviceBusChange::Opened(device),
        ]);
        let mut expected = HypercallAnswer::lv1_status(0, [0; 7]);
        expected.push_change(DeviceBusChange::Closed(device));
        assert_eq!(
            close(&state, 1, device.device),
            HypercallDisposition::Answered(expected),
            "{device:?}"
        );
    }
}

/// [OFW-4.93 lv1 a:0x2364B8-0x236538] gives -11 for a clear open flag;
/// [OFW-4.93 lv1 a:0x2373EC] returns before the hook and the close entry.
#[test]
fn the_close_of_a_device_of_bus_1_that_is_not_open_gets_wrong_state_and_no_change() {
    let others_open = state_with(&[
        DeviceBusChange::Opened(DEVICE_1),
        DeviceBusChange::DmaRegionAllocated(REGION),
    ]);
    for state in [DeviceBusState::default(), others_open] {
        for device in [0, 2] {
            assert_eq!(
                close(&state, 1, device),
                answered(0x0000_0000_FFFF_FFF5),
                "device {device}"
            );
        }
    }
}

/// The access check gives -11 for device 1 on the terms of device 0
/// [OFW-4.93 lv1 a:0x2364B8-0x236538].
#[test]
fn the_close_of_device_1_that_is_not_open_gets_wrong_state_and_no_change() {
    let others_open = state_with(&[
        DeviceBusChange::Opened(NETWORK_DEVICE),
        DeviceBusChange::Opened(DEVICE_2),
    ]);
    for state in [DeviceBusState::default(), others_open] {
        assert_eq!(close(&state, 1, 1), answered(0x0000_0000_FFFF_FFF5));
    }
}

/// A close takes the device out of the open set, so a second close fails
/// the access check [OFW-4.93 lv1 a:0x236534-0x236538].
#[test]
fn a_second_close_gets_wrong_state() {
    let mut state = state_with(&[DeviceBusChange::Opened(NETWORK_DEVICE)]);
    let HypercallDisposition::Answered(first) = close(&state, 1, 0) else {
        panic!("the first close is answered");
    };
    assert_eq!(first.r3, 0);
    state.apply(DeviceBusChange::Closed(NETWORK_DEVICE));
    assert_eq!(close(&state, 1, 0), answered(0x0000_0000_FFFF_FFF5));
}

/// The handler [OFW-4.93 lv1 a:0x30701C-0x30702C] and the access check
/// [OFW-4.93 lv1 a:0x236420-0x236428] [OFW-4.93 lv1 a:0x236554-0x236558]
/// answer these calls before any state is read.
#[test]
fn each_check_that_reads_no_state_gets_its_marked_status() {
    let open = state_with(&[DeviceBusChange::Opened(NETWORK_DEVICE)]);
    let cases = [
        (5, 0, 0x0000_0001_FFFF_FFEF),
        (u64::MAX, 0, 0x0000_0001_FFFF_FFEF),
        (0, 0, 0x0000_0001_FFFF_FFFA),
        (0, 7, 0x0000_0001_FFFF_FFFA),
        (4, 7, 0x0000_0002_FFFF_FFFA),
        (4, u64::MAX - 1, 0x0000_0002_FFFF_FFFA),
    ];
    assert_eq!(cases.len(), 6);
    for state in [DeviceBusState::default(), open] {
        for (bus, device, r3) in cases {
            assert_eq!(
                close(&state, bus, device),
                answered(r3),
                "bus {bus:#x} device {device:#x}"
            );
        }
    }
}

/// The storage bus object's method at +48 returns 0 for a partition that
/// holds no slot [OFW-4.93 lv1 a:0x25477C-0x2547FC], so the access check
/// gives -11 [OFW-4.93 lv1 a:0x23650C-0x236538].
#[test]
fn the_close_of_a_storage_bus_that_is_not_open_gets_wrong_state() {
    assert_eq!(
        close(&DeviceBusState::default(), 4, u64::MAX),
        answered(0x0000_0000_FFFF_FFF5)
    );
}

#[test]
fn the_close_of_a_storage_bus_that_is_open_is_a_refusal() {
    let state = state_with(&[DeviceBusChange::StorageBusOpened(StorageBusOpen { r5: 0 })]);
    let call = request(4, u64::MAX);
    assert_eq!(
        close_device(&call, Some("4.93"), &state),
        refused(&call, STORAGE_BUS.what)
    );
}

#[test]
fn a_close_of_a_device_that_lv1_finds_through_unmodelled_state_is_a_refusal() {
    let cases = [(1, 3), (1, 7), (2, 0), (3, 1), (4, 0), (4, 6)];
    assert_eq!(cases.len(), 6);
    for (bus, device) in cases {
        let call = request(bus, device);
        assert_eq!(
            close_device(&call, Some("4.93"), &DeviceBusState::default()),
            refused(&call, OTHER_DEVICE.what),
            "bus {bus} device {device}"
        );
    }
}

#[test]
fn a_close_at_a_firmware_whose_handler_is_not_read_is_a_refusal() {
    let state = state_with(&[DeviceBusChange::Opened(NETWORK_DEVICE)]);
    let call = request(1, 0);
    for firmware in [None, Some("4.92"), Some("3.70")] {
        assert_eq!(
            close_device(&call, firmware, &state),
            refused(&call, UNREAD_FIRMWARE.what),
            "{firmware:?}"
        );
    }
}

/// The close entry returns 0 after the free step whatever the regions
/// gave [OFW-4.93 lv1 a:0x229610-0x229614] [OFW-4.93 lv1 a:0x22968C-0x2296A0].
#[test]
fn a_held_region_and_mapping_leave_the_close_answer_as_it_is() {
    let state = state_with(&[
        DeviceBusChange::Opened(NETWORK_DEVICE),
        DeviceBusChange::DmaRegionAllocated(REGION),
        DeviceBusChange::DmaRegionMapped(MAPPING),
    ]);
    let HypercallDisposition::Answered(answer) = close(&state, 1, 0) else {
        panic!("the close is answered");
    };
    assert_eq!(answer.r3, 0);
    let mut expected = HypercallAnswer::lv1_status(0, [0; 7]);
    expected.push_change(DeviceBusChange::Closed(NETWORK_DEVICE));
    expected.push_change(NetworkChange::TornDown);
    assert_eq!(answer, expected);
}
