use super::*;
use cellgov_event::UnitId;
use cellgov_ps3_abi::lv1::storage_registration::registration;
use cellgov_time::GuestTicks;

/// The `sc` of the 4.93 kernel's storage device open
/// [OFW-4.93 lv2_kernel a:0x80000000000ac120].
const KERNEL_SC: u64 = 0x8000_0000_000A_C120;

/// The r5 of the kernel's storage device open: -1 with all but the top
/// bit cleared [OFW-4.93 lv2_kernel a:0x80000000000ac0fc]
/// [OFW-4.93 lv2_kernel a:0x80000000000ac118].
const KERNEL_R5: u64 = 0x8000_0000_0000_0000;

fn request(bus: u64, device: u64, r5: u64) -> HypercallRequest {
    let mut inputs = [0; 8];
    inputs[..3].copy_from_slice(&[bus, device, r5]);
    HypercallRequest {
        unit: UnitId::new(0),
        number: OPEN_DEVICE,
        inputs,
        pc: KERNEL_SC,
        lr: 0,
        thread: Some(0),
        problem_state: false,
        tick: GuestTicks::new(35_444),
    }
}

/// The device ids of the 4.93 storage bus table, in table order.
fn table_devices() -> Vec<u64> {
    let devices: Vec<u64> = registration("4.93")
        .expect("the 4.93 registration")
        .devices
        .iter()
        .map(|device| device.id)
        .collect();
    assert_eq!(devices, [0, 1, 2, 3, 4]);
    devices
}

fn held(device: u64, r5: u64) -> DeviceBusState {
    let mut state = DeviceBusState::default();
    state.apply(DeviceBusChange::StorageDeviceOpened(StorageDeviceOpen {
        device,
        r5,
    }));
    state
}

fn opened(device: u64, r5: u64) -> HypercallDisposition {
    let mut answer = HypercallAnswer::lv1_status(0, [0; 7]);
    answer.push_change(DeviceBusChange::StorageDeviceOpened(StorageDeviceOpen {
        device,
        r5,
    }));
    HypercallDisposition::Answered(answer)
}

#[test]
fn a_first_open_of_each_table_device_answers_0_and_keeps_r5() {
    for device in table_devices() {
        for r5 in [0, KERNEL_R5] {
            assert_eq!(
                open_device(
                    &request(4, device, r5),
                    Some("4.93"),
                    &DeviceBusState::default()
                ),
                opened(device, r5),
                "device {device} r5 {r5:#x}"
            );
        }
    }
}

/// The partition check returns -9 for a used slot that holds the caller's
/// partition [OFW-4.93 lv1 a:0x254734], sign-extended
/// [OFW-4.93 lv1 a:0x254678], and the open function clears the high word
/// [OFW-4.93 lv1 a:0x237630].
#[test]
fn an_open_of_a_held_table_device_gets_busy_and_no_change() {
    let busy =
        HypercallDisposition::Answered(HypercallAnswer::lv1_status(0x0000_0000_FFFF_FFF7, [0; 7]));
    for device in table_devices() {
        assert_eq!(
            open_device(
                &request(4, device, KERNEL_R5),
                Some("4.93"),
                &held(device, KERNEL_R5)
            ),
            busy,
            "device {device}"
        );
    }
}

#[test]
fn the_open_of_one_table_device_leaves_the_others_openable() {
    let state = held(0, KERNEL_R5);
    for device in table_devices().into_iter().filter(|&device| device != 0) {
        assert_eq!(
            open_device(&request(4, device, KERNEL_R5), Some("4.93"), &state),
            opened(device, KERNEL_R5),
            "device {device}"
        );
    }
}

#[test]
fn applying_the_open_records_the_device_and_its_r5() {
    let state = held(3, KERNEL_R5);
    assert_eq!(
        state.storage_device_open(3),
        Some(StorageDeviceOpen {
            device: 3,
            r5: KERNEL_R5
        })
    );
    assert_eq!(state.storage_device_open(0), None);
    assert_eq!(state.storage_device_opens().count(), 1);
    assert_eq!(state.open_devices().count(), 0);
    assert_eq!(state.storage_bus_open(), None);
}
