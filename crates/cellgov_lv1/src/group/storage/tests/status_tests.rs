use super::*;
use crate::group::state::StateChange;
use crate::group::storage::NotifyRequest;
use crate::Hypervisor;
use cellgov_event::UnitId;
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_ps3_abi::lv1::call::{OPEN_DEVICE, STORAGE_WRITE};
use cellgov_time::GuestTicks;

/// The `sc` of the kernel's `open_device` of the storage bus
/// [OFW-4.93 lv2_kernel a:0x80000000000AE110].
const KERNEL_OPEN_SC: u64 = 0x8000_0000_000A_E110;

/// The `sc` of the kernel's notification write
/// [OFW-4.93 lv2_kernel a:0x80000000000AE250].
const KERNEL_WRITE_SC: u64 = 0x8000_0000_000A_E250;

/// The `sc` of the kernel's status call in its storage interrupt
/// function [OFW-4.93 lv2_kernel a:0x80000000000AE524].
const KERNEL_SC: u64 = 0x8000_0000_000A_E524;

/// The address of the notification write's buffer, in the real mode
/// region.
const BUFFER_ADDRESS: u64 = 0x1_0000;

/// The kernel's notification write: device -1, region 0, start sector 0,
/// one sector, flags 0 and the buffer
/// [OFW-4.93 lv2_kernel a:0x80000000000AE234-0x80000000000AE24C].
const WRITE_INPUTS: [u64; 6] = [u64::MAX, 0, 0, 1, 0, BUFFER_ADDRESS];

/// The kernel's r3: the notification device
/// [OFW-4.93 lv2_kernel a:0x80000000000AE520].
const NOTIFICATION_DEVICE: u64 = u64::MAX;

/// The r5 of an open that makes lv1 read the slot word: the r5 that the
/// kernel's open of one storage device passes
/// [OFW-4.93 lv2_kernel a:0x80000000000AC0FC-0x80000000000AC120].
const SLOT_WORD_R5: u64 = 0x8000_0000_0000_0000;

/// Memory whose buffer holds the kernel's command 0 and mask 2
/// [OFW-4.93 lv2_kernel a:0x80000000000AE1DC-0x80000000000AE204].
fn write_memory() -> GuestMemory {
    let mut memory = GuestMemory::new(0x2_0000);
    let head = ByteRange::new(GuestAddr::new(BUFFER_ADDRESS), 16).expect("the buffer head");
    let mut bytes = 0u64.to_be_bytes().to_vec();
    bytes.extend_from_slice(&2u64.to_be_bytes());
    memory.apply_commit(head, &bytes).expect("the buffer write");
    memory
}

fn call(number: u64, pc: u64, thread: u8, args: &[u64]) -> HypercallRequest {
    let mut inputs = [0; 8];
    inputs[..args.len()].copy_from_slice(args);
    HypercallRequest {
        unit: UnitId::new(u64::from(thread)),
        number,
        inputs,
        pc,
        lr: 0,
        thread: Some(thread),
        problem_state: false,
        tick: GuestTicks::new(34_217),
    }
}

/// The kernel's status call for device `device`, from hardware thread 1.
fn get_status(device: u64) -> HypercallRequest {
    call(STORAGE_GET_ASYNC_STATUS, KERNEL_SC, 1, &[device])
}

/// A 4.93 hypervisor after the kernel's open of the storage bus with r5
/// = `r5` when `open` is set, and after `writes` notification writes.
fn hypervisor(open: Option<u64>, writes: u8) -> Hypervisor {
    let mut hv = Hypervisor::new();
    hv.set_firmware("4.93");
    if let Some(r5) = open {
        let opened = hv.dispatch(&call(OPEN_DEVICE, KERNEL_OPEN_SC, 0, &[4, u64::MAX, r5]));
        assert_eq!(answered(opened).r3, 0);
    }
    let memory = write_memory();
    for _ in 0..writes {
        let written = hv.dispatch_with(
            &call(STORAGE_WRITE, KERNEL_WRITE_SC, 0, &WRITE_INPUTS),
            &memory,
        );
        assert_eq!(answered(written).r3, 0);
    }
    hv
}

fn answered(disposition: HypercallDisposition) -> HypercallAnswer {
    match disposition {
        HypercallDisposition::Answered(answer) => answer,
        refused => panic!("{refused:?}"),
    }
}

#[test]
fn the_kernels_status_call_takes_the_completed_request_of_tag_1_and_frees_the_tag() {
    let mut hv = hypervisor(Some(0), 1);
    assert_eq!(hv.groups().storage().held_tags(), 0b10);
    let answer = answered(hv.dispatch(&get_status(NOTIFICATION_DEVICE)));
    assert_eq!(answer.r3, 0);
    assert_eq!(answer.outputs(), Some([1, 0, 0, 0, 0, 0, 0]));
    assert_eq!(
        answer.changes,
        vec![StateChange::from(StorageChange::NotifyRequestTaken(1))]
    );
    let storage = hv.groups().storage();
    assert_eq!(storage.notify_requests().count(), 0);
    assert_eq!(storage.held_tags(), 0);
    assert_eq!(storage.notify_event_mask(), Some(2));
}

#[test]
fn the_kernels_next_status_call_with_no_completed_request_gets_no_entry() {
    let mut hv = hypervisor(Some(0), 1);
    answered(hv.dispatch(&get_status(NOTIFICATION_DEVICE)));
    let answer = answered(hv.dispatch(&get_status(NOTIFICATION_DEVICE)));
    assert_eq!(answer.r3, zero_extended(ASYNC_STATUS_NONE_STATUS));
    assert_eq!(answer.r3, 0xFFFF_FFFA);
    assert_eq!(answer.outputs(), Some([0; 7]));
    assert_eq!(answer.changes, vec![]);
}

#[test]
fn each_status_call_takes_the_lowest_completed_tag_first() {
    let mut hv = hypervisor(Some(0), 3);
    for tag in 1..=3u64 {
        let answer = answered(hv.dispatch(&get_status(NOTIFICATION_DEVICE)));
        assert_eq!((answer.r3, answer.outputs().map(|o| o[0])), (0, Some(tag)));
    }
    assert_eq!(hv.groups().storage().held_tags(), 0);
}

#[test]
fn a_freed_tag_is_the_next_notification_writes_tag() {
    let mut hv = hypervisor(Some(0), 1);
    answered(hv.dispatch(&get_status(NOTIFICATION_DEVICE)));
    let written = answered(hv.dispatch_with(
        &call(STORAGE_WRITE, KERNEL_WRITE_SC, 0, &WRITE_INPUTS),
        &write_memory(),
    ));
    assert_eq!(written.outputs().map(|o| o[0]), Some(1));
}

#[test]
fn a_waiting_request_is_not_taken() {
    let hv = hypervisor(Some(0), 0);
    let mut requests = StorageState::default();
    requests.apply(StorageChange::NotifyRequested(NotifyRequest {
        tag: 1,
        completion: None,
    }));
    let request = get_status(NOTIFICATION_DEVICE);
    assert_eq!(
        storage_get_async_status(&request, Some("4.93"), &requests, hv.groups().device_bus()),
        lv1_answer(ASYNC_STATUS_NONE_STATUS)
    );
}

#[test]
fn a_completed_request_keeps_its_status_in_r5() {
    let hv = hypervisor(Some(0), 0);
    let mut requests = StorageState::default();
    for (tag, completion) in [(2, None), (5, Some(0x1234))] {
        requests.apply(StorageChange::NotifyRequested(NotifyRequest {
            tag,
            completion,
        }));
    }
    let request = get_status(NOTIFICATION_DEVICE);
    let answer = answered(storage_get_async_status(
        &request,
        Some("4.93"),
        &requests,
        hv.groups().device_bus(),
    ));
    assert_eq!(answer.outputs(), Some([5, 0x1234, 0, 0, 0, 0, 0]));
}

#[test]
fn a_partition_that_did_not_open_the_bus_gets_wrong_state() {
    let mut hv = hypervisor(None, 0);
    assert_eq!(
        hv.dispatch(&get_status(NOTIFICATION_DEVICE)),
        lv1_answer(ASYNC_STATUS_NOT_READY_STATUS)
    );
}

/// lv1 compares the whole doubleword of the device id: unsigned with 6
/// [OFW-4.93 lv1 a:0x251EAC] and signed with -1 [OFW-4.93 lv1 a:0x251EB8].
/// So a low word of -1 and the sign bit alone are devices above 6.
#[test]
fn a_device_above_6_other_than_minus_1_gets_no_entry() {
    let mut hv = hypervisor(Some(0), 1);
    for device in [
        STORAGE_DEVICE_ID_MAX + 1,
        u64::MAX - 1,
        0xFFFF_FFFF,
        1 << 63,
    ] {
        assert_eq!(
            hv.dispatch(&get_status(device)),
            lv1_answer(ASYNC_STATUS_NO_DEVICE_STATUS),
            "{device:#x}"
        );
    }
    assert_eq!(hv.groups().storage().held_tags(), 0b10);
}

#[test]
fn a_device_from_0_to_6_is_a_refusal() {
    let mut hv = hypervisor(Some(0), 1);
    for device in [0, STORAGE_DEVICE_ID_MAX] {
        let request = get_status(device);
        assert_eq!(hv.dispatch(&request), DEVICE_OBJECT.refusal(&request));
    }
}

#[test]
fn an_open_that_reads_the_slot_word_is_a_refusal_once_no_request_is_completed() {
    let mut hv = hypervisor(Some(SLOT_WORD_R5), 1);
    let taken = answered(hv.dispatch(&get_status(NOTIFICATION_DEVICE)));
    assert_eq!(taken.outputs().map(|o| o[0]), Some(1));
    let request = get_status(NOTIFICATION_DEVICE);
    assert_eq!(hv.dispatch(&request), SLOT_WORD.refusal(&request));
}

#[test]
fn a_status_call_at_a_firmware_whose_handler_is_not_read_is_a_refusal() {
    let mut hv = Hypervisor::new();
    hv.set_firmware("4.92");
    let request = get_status(NOTIFICATION_DEVICE);
    assert_eq!(hv.dispatch(&request), UNREAD_FIRMWARE.refusal(&request));
}

#[test]
fn taking_the_request_removes_its_term_from_the_sync_state() {
    let mut taken = StorageState::default();
    let mut untouched = StorageState::default();
    for state in [&mut taken, &mut untouched] {
        state.apply(StorageChange::NotifyEventMaskSet(2));
    }
    taken.apply(StorageChange::NotifyRequested(NotifyRequest {
        tag: 1,
        completion: Some(0),
    }));
    assert_ne!(taken.sync_term(), untouched.sync_term());
    taken.apply(StorageChange::NotifyRequestTaken(1));
    assert_eq!(taken, untouched);
    assert_eq!(taken.sync_term(), untouched.sync_term());
}
