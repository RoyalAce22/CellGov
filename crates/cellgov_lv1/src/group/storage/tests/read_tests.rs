use super::*;
use crate::group::memory::storage::allocated_storage;
use crate::group::state::StateChange;
use crate::Hypervisor;
use cellgov_event::UnitId;
use cellgov_mem::{GuestMemory, PageSize, Region};
use cellgov_ps3_abi::lv1::call::{
    ALLOCATE_MEMORY, OPEN_DEVICE, STORAGE_GET_ASYNC_STATUS, STORAGE_WRITE,
};
use cellgov_ps3_abi::lv1::storage::{
    NOTIFY_REQUEST_TAGS, NOTIFY_WRITE_BUFFER_PAST_REGION_STATUS, NOTIFY_WRITE_COMMAND,
};
use cellgov_ps3_abi::lv1::storage_read::NOTIFY_READ_ENTRY_OFFSET;
use cellgov_time::GuestTicks;

/// The `sc` of the kernel's `open_device` of the storage bus
/// [OFW-4.93 lv2_kernel a:0x80000000000AE110].
const KERNEL_OPEN_SC: u64 = 0x8000_0000_000A_E110;

/// The `sc` of the kernel's notification write
/// [OFW-4.93 lv2_kernel a:0x80000000000AE250].
const KERNEL_WRITE_SC: u64 = 0x8000_0000_000A_E250;

/// The `sc` of the kernel's notification read
/// [OFW-4.93 lv2_kernel a:0x80000000000AE408].
const KERNEL_READ_SC: u64 = 0x8000_0000_000A_E408;

/// The address of the write's buffer, in the real mode region.
const WRITE_BUFFER: u64 = 0x1_0000;

/// The address of the read's buffer, in the real mode region.
const READ_BUFFER: u64 = 0x1_1000;

/// The bytes of the caller's real address space in these tests.
const MEMORY_BYTES: usize = 0x2_0000;

/// The kernel's r3 to r8 for the read: device -1, region 0, start sector
/// 0, one sector, flags 0 and the buffer
/// [OFW-4.93 lv2_kernel a:0x80000000000AE3EC-0x80000000000AE404].
const KERNEL_READ: [u64; 6] = [u64::MAX, 0, 0, 1, 0, READ_BUFFER];

/// The event mask that the kernel stores at +8 of its write's buffer
/// [OFW-4.93 lv2_kernel a:0x80000000000AE1F4-0x80000000000AE200].
const KERNEL_MASK: u64 = 2;

/// Memory whose write buffer holds the command 0 and `mask`.
fn memory(mask: u64) -> GuestMemory {
    let mut memory = GuestMemory::new(MEMORY_BYTES);
    let head = ByteRange::new(GuestAddr::new(WRITE_BUFFER), 16).expect("the buffer head");
    let mut bytes = NOTIFY_WRITE_COMMAND.to_be_bytes().to_vec();
    bytes.extend_from_slice(&mask.to_be_bytes());
    memory.apply_commit(head, &bytes).expect("the buffer write");
    memory
}

fn call(number: u64, pc: u64, args: &[u64]) -> HypercallRequest {
    let mut inputs = [0; 8];
    inputs[..args.len()].copy_from_slice(args);
    HypercallRequest {
        unit: UnitId::new(1),
        number,
        inputs,
        pc,
        lr: 0,
        thread: Some(1),
        problem_state: false,
        tick: GuestTicks::new(34_296),
    }
}

fn read(args: &[u64]) -> HypercallRequest {
    call(STORAGE_READ, KERNEL_READ_SC, args)
}

fn answered(disposition: HypercallDisposition) -> HypercallAnswer {
    match disposition {
        HypercallDisposition::Answered(answer) => answer,
        refused => panic!("{refused:?}"),
    }
}

/// lv1's `status` in r3, zero-extended as the handler returns it
/// [OFW-4.93 lv1 a:0x308CD8], with r4 to r10 cleared.
fn status(status: i64) -> HypercallDisposition {
    HypercallDisposition::Answered(HypercallAnswer::lv1_status(zero_extended(status), []))
}

/// A 4.93 hypervisor after these kernel calls:
/// - the open of the storage bus;
/// - the notification write with `mask`;
/// - the status call that takes the write's request, so tag 1 is free.
fn after_write(mask: u64, memory: &GuestMemory) -> Hypervisor {
    let mut hv = Hypervisor::new();
    hv.set_firmware("4.93");
    let open = call(OPEN_DEVICE, KERNEL_OPEN_SC, &[4, u64::MAX, 0]);
    assert_eq!(answered(hv.dispatch_with(&open, memory)).r3, 0, "open");
    let write = call(
        STORAGE_WRITE,
        KERNEL_WRITE_SC,
        &[u64::MAX, 0, 0, 1, 0, WRITE_BUFFER],
    );
    assert_eq!(answered(hv.dispatch_with(&write, memory)).r3, 0, "write");
    take_completed(&mut hv, memory);
    assert_eq!(hv.groups().storage().notify_event_mask(), Some(mask));
    hv
}

/// The status call that takes the lowest completed request.
fn take_completed(hv: &mut Hypervisor, memory: &GuestMemory) {
    let status = call(STORAGE_GET_ASYNC_STATUS, KERNEL_WRITE_SC, &[u64::MAX]);
    assert_eq!(answered(hv.dispatch_with(&status, memory)).r3, 0, "status");
}

/// The bytes that a report of `device` writes at the read's buffer.
fn report_bytes(device: &StorageDevice) -> Vec<u8> {
    std::iter::once(NOTIFY_READ_DEVICE_EVENT)
        .chain(device.notify_entry())
        .flat_map(u64::to_be_bytes)
        .collect()
}

fn table() -> &'static [StorageDevice] {
    registration("4.93").expect("the 4.93 registration").devices
}

#[test]
fn the_first_read_registers_and_reports_device_0_and_completes_under_tag_1() {
    let memory = memory(KERNEL_MASK);
    let mut hv = after_write(KERNEL_MASK, &memory);
    let answer = answered(hv.dispatch_with(&read(&KERNEL_READ), &memory));
    assert_eq!(answer.r3, 0);
    assert_eq!(answer.outputs(), Some([1, 0, 0, 0, 0, 0, 0]));
    assert_eq!(
        answer.changes,
        vec![
            StateChange::from(StorageChange::DeviceRegistered(0)),
            StateChange::from(StorageChange::NotifyDeviceReported),
            StateChange::from(StorageChange::NotifyRequested(NotifyRequest {
                tag: 1,
                completion: Some(0),
            })),
        ]
    );
    let device = &table()[0];
    assert_eq!(
        answer.effects,
        vec![Effect::SharedWriteIntent {
            range: ByteRange::new(GuestAddr::new(READ_BUFFER), 40).expect("the report"),
            bytes: WritePayload::new(report_bytes(device)),
            source: UnitId::new(1),
            source_time: GuestTicks::new(34_296),
        }]
    );
    let storage = hv.groups().storage();
    assert_eq!(storage.reported_devices(), 1);
    assert_eq!(storage.registered_devices().collect::<Vec<_>>(), [0]);
}

#[test]
fn the_report_bytes_are_the_device_event_then_the_table_entry_at_offset_8() {
    let bytes = report_bytes(&table()[3]);
    let word = |at: usize| u64::from_be_bytes(bytes[at..at + 8].try_into().expect("8 bytes"));
    let entry = NOTIFY_READ_ENTRY_OFFSET as usize;
    assert_eq!(bytes.len(), 40);
    assert_eq!(word(0), 1);
    assert_eq!(
        [
            word(entry),
            word(entry + 8),
            word(entry + 16),
            word(entry + 24)
        ],
        [4, 3, 0, 2]
    );
}

#[test]
fn five_reads_report_the_table_in_order_and_the_sixth_waits_under_tag_1() {
    let memory = memory(KERNEL_MASK);
    let mut hv = after_write(KERNEL_MASK, &memory);
    let devices = table();
    assert_eq!(devices.len(), 5);
    for (index, device) in devices.iter().enumerate() {
        let answer = answered(hv.dispatch_with(&read(&KERNEL_READ), &memory));
        assert_eq!(answer.outputs().map(|o| o[0]), Some(1), "read {index}");
        let Some(Effect::SharedWriteIntent { bytes, .. }) = answer.effects.first() else {
            panic!("read {index}: {:?}", answer.effects);
        };
        assert_eq!(bytes.bytes(), report_bytes(device), "read {index}");
        take_completed(&mut hv, &memory);
    }
    let answer = answered(hv.dispatch_with(&read(&KERNEL_READ), &memory));
    assert_eq!(answer.r3, 0);
    assert_eq!(answer.outputs(), Some([1, 0, 0, 0, 0, 0, 0]));
    assert!(answer.effects.is_empty(), "{:?}", answer.effects);
    let waiting = WaitingRead {
        buffer: READ_BUFFER,
        tag: 1,
    };
    assert_eq!(
        answer.changes,
        vec![
            StateChange::from(StorageChange::NotifyRequested(NotifyRequest {
                tag: 1,
                completion: None,
            })),
            StateChange::from(StorageChange::NotifyReadWaiting(waiting)),
        ]
    );
    let storage = hv.groups().storage();
    assert_eq!(storage.reported_devices(), 5);
    assert_eq!(storage.waiting_read(), Some(waiting));
    assert_eq!(
        storage.registered_devices().collect::<Vec<_>>(),
        [0, 1, 2, 3, 4]
    );
}

#[test]
fn a_read_before_any_write_has_mask_0_and_waits() {
    let memory = memory(KERNEL_MASK);
    let mut hv = Hypervisor::new();
    hv.set_firmware("4.93");
    let open = call(OPEN_DEVICE, KERNEL_OPEN_SC, &[4, u64::MAX, 0]);
    assert_eq!(answered(hv.dispatch_with(&open, &memory)).r3, 0);
    let answer = answered(hv.dispatch_with(&read(&KERNEL_READ), &memory));
    assert_eq!(answer.r3, 0);
    assert_eq!(answer.outputs().map(|o| o[0]), Some(1));
    assert_eq!(
        hv.groups().storage().waiting_read(),
        Some(WaitingRead {
            buffer: READ_BUFFER,
            tag: 1,
        })
    );
    assert_eq!(hv.groups().storage().reported_devices(), 0);
}

#[test]
fn a_device_above_6_other_than_minus_1_gets_no_entry() {
    let memory = memory(KERNEL_MASK);
    let mut hv = after_write(KERNEL_MASK, &memory);
    for device in [7, u64::MAX - 1] {
        let mut inputs = KERNEL_READ;
        inputs[0] = device;
        assert_eq!(
            hv.dispatch_with(&read(&inputs), &memory),
            status(STORAGE_READ_NO_DEVICE_STATUS),
            "device {device:#x}"
        );
    }
}

#[test]
fn a_device_from_0_to_6_is_a_refusal() {
    let memory = memory(KERNEL_MASK);
    let mut hv = after_write(KERNEL_MASK, &memory);
    for device in [0, STORAGE_DEVICE_ID_MAX] {
        let mut inputs = KERNEL_READ;
        inputs[0] = device;
        let request = read(&inputs);
        assert_eq!(
            hv.dispatch_with(&request, &memory),
            DEVICE_OBJECT.refusal(&request)
        );
    }
}

#[test]
fn a_partition_that_did_not_open_the_bus_gets_wrong_state() {
    let memory = memory(KERNEL_MASK);
    let mut hv = Hypervisor::new();
    hv.set_firmware("4.93");
    assert_eq!(
        hv.dispatch_with(&read(&KERNEL_READ), &memory),
        status(STORAGE_READ_NOT_READY_STATUS)
    );
}

#[test]
fn a_start_sector_count_or_flags_that_lv1_does_not_accept_gets_illegal_parameter() {
    let memory = memory(KERNEL_MASK);
    let mut hv = after_write(KERNEL_MASK, &memory);
    for (index, value) in [(2, 1), (3, 0), (3, 2), (4, 8)] {
        let mut inputs = KERNEL_READ;
        inputs[index] = value;
        assert_eq!(
            hv.dispatch_with(&read(&inputs), &memory),
            status(NOTIFY_READ_BAD_REQUEST_STATUS),
            "r{} = {value}",
            index + 3
        );
    }
    assert_eq!(hv.groups().storage().reported_devices(), 0);
}

#[test]
fn a_read_while_every_tag_is_held_gets_busy() {
    let memory = memory(0);
    let mut hv = after_write(0, &memory);
    for tag in 1..NOTIFY_REQUEST_TAGS {
        let answer = answered(hv.dispatch_with(&read(&KERNEL_READ), &memory));
        assert_eq!(answer.outputs().map(|o| o[0]), Some(u64::from(tag)));
    }
    assert_eq!(
        hv.dispatch_with(&read(&KERNEL_READ), &memory),
        status(NOTIFY_REQUESTS_FULL_STATUS)
    );
}

#[test]
fn a_mask_with_bit_0_is_a_refusal() {
    let memory = memory(KERNEL_MASK | 1);
    let mut hv = after_write(KERNEL_MASK | 1, &memory);
    let request = read(&KERNEL_READ);
    assert_eq!(
        hv.dispatch_with(&request, &memory),
        TYPE0_EVENTS.refusal(&request)
    );
}

#[test]
fn a_mask_with_bit_2_reports_devices_first_and_then_is_a_refusal() {
    let mask = KERNEL_MASK | 4;
    let memory = memory(mask);
    let mut hv = after_write(mask, &memory);
    for index in 0..table().len() {
        let answer = answered(hv.dispatch_with(&read(&KERNEL_READ), &memory));
        assert_eq!(answer.effects.len(), 1, "read {index}");
        take_completed(&mut hv, &memory);
    }
    let request = read(&KERNEL_READ);
    assert_eq!(
        hv.dispatch_with(&request, &memory),
        QUEUED_EVENTS.refusal(&request)
    );
}

#[test]
fn a_buffer_that_the_memory_does_not_hold_is_a_refusal() {
    let memory = memory(KERNEL_MASK);
    let mut hv = after_write(KERNEL_MASK, &memory);
    let mut inputs = KERNEL_READ;
    inputs[5] = MEMORY_BYTES as u64 - NOTIFY_BUFFER_BYTES + 8;
    let request = read(&inputs);
    assert_eq!(
        hv.dispatch_with(&request, &memory),
        BUFFER.refusal(&request)
    );
}

/// The size and the page size exponent of the kernel's allocation at
/// [OFW-4.93 lv2_kernel a:0x80000000002653c0], which passes 20.
const REGION_SIZE: u64 = 0x10_0000;
const REGION_PAGE_SIZE_EXP: u64 = 20;

#[test]
fn a_buffer_that_ends_past_its_allocated_region_gets_no_privilege_and_keeps_its_tag() {
    let mut memory = GuestMemory::from_regions(vec![
        Region::new(0, MEMORY_BYTES, "flat", PageSize::Page64K),
        allocated_storage(),
    ])
    .expect("the regions do not overlap");
    let head = ByteRange::new(GuestAddr::new(WRITE_BUFFER), 16).expect("the buffer head");
    let mut bytes = NOTIFY_WRITE_COMMAND.to_be_bytes().to_vec();
    bytes.extend_from_slice(&KERNEL_MASK.to_be_bytes());
    memory.apply_commit(head, &bytes).expect("the buffer write");
    let mut hv = after_write(KERNEL_MASK, &memory);
    let allocate = call(
        ALLOCATE_MEMORY,
        KERNEL_WRITE_SC,
        &[REGION_SIZE, REGION_PAGE_SIZE_EXP, 0, 0],
    );
    let lpar = answered(hv.dispatch_with(&allocate, &memory))
        .outputs()
        .expect("outputs")[0];
    let mut inputs = KERNEL_READ;
    inputs[5] = lpar + REGION_SIZE - NOTIFY_BUFFER_BYTES + 8;
    let answer = answered(hv.dispatch_with(&read(&inputs), &memory));
    assert_eq!(
        answer.r3,
        zero_extended(NOTIFY_WRITE_BUFFER_PAST_REGION_STATUS)
    );
    assert_eq!(answer.outputs(), Some([1, 0, 0, 0, 0, 0, 0]));
    assert_eq!(
        answer.changes,
        vec![StateChange::from(StorageChange::NotifyRequested(
            NotifyRequest {
                tag: 1,
                completion: None,
            }
        ))]
    );
    let next = answered(hv.dispatch_with(&read(&KERNEL_READ), &memory));
    assert_eq!(next.outputs().map(|o| o[0]), Some(2));
    assert_eq!(hv.groups().storage().waiting_read(), None);
}

#[test]
fn a_read_at_a_firmware_whose_handler_is_not_read_is_a_refusal() {
    let memory = memory(KERNEL_MASK);
    let mut hv = Hypervisor::new();
    hv.set_firmware("4.92");
    let request = read(&KERNEL_READ);
    assert_eq!(
        hv.dispatch_with(&request, &memory),
        UNREAD_FIRMWARE.refusal(&request)
    );
}
