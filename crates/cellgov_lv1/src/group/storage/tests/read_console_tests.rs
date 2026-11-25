//! The 4.93 hypervisor's answers to the kernel's notification reads, held
//! to the `lv1_storage_registration` capture of the game OS record and the
//! bus table, and to the `lv1_storage_notify_request` capture that names
//! the record's partition and event mask.

use super::*;
use crate::Hypervisor;
use cellgov_event::UnitId;
use cellgov_mem::GuestMemory;
use cellgov_ps3_abi::lv1::call::{OPEN_DEVICE, STORAGE_GET_ASYNC_STATUS, STORAGE_WRITE};
use cellgov_ps3_abi::lv1::storage::{NOTIFY_REQUEST_CAPTURE, NOTIFY_WRITE_COMMAND};
use cellgov_ps3_abi::lv1::storage_registration::REGISTRATION_CAPTURE;
use cellgov_time::GuestTicks;

/// The global that holds the storage bus object, from the TOC slot at
/// r2 - 23408 [OFW-4.93 lv1 a:0x2514C8] [OFW-4.93 lv1 a:0x3544C8-0x3544CC].
const STORAGE_BUS_GLOBAL: u64 = 0x36_8388;

/// The offset in the bus object of the list's last open record
/// [OFW-4.93 lv1 a:0x259218-0x259244].
const LAST_RECORD_OFFSET: u64 = 3552 + 8;

/// The offset in the bus object of slot 1's partition
/// [OFW-4.93 lv1 a:0x2547B0].
const SLOT_1_OFFSET: u64 = 2608;

/// The offsets in an open record of its partition and its event mask
/// [OFW-4.93 lv1 a:0x25937C] [OFW-4.93 lv1 a:0x259464].
const RECORD_PARTITION_OFFSET: u64 = 40;
const RECORD_MASK_OFFSET: u64 = 48;

/// The offsets in an open record of the type-0 index, the report index,
/// the waiting buffer and the waiting tag
/// [OFW-4.93 lv1 a:0x259478] [OFW-4.93 lv1 a:0x259530]
/// [OFW-4.93 lv1 a:0x25970C-0x259710].
const RECORD_TYPE0_INDEX_OFFSET: u64 = 56;
const RECORD_REPORT_INDEX_OFFSET: u64 = 64;
const RECORD_WAITING_BUFFER_OFFSET: u64 = 72;
const RECORD_WAITING_TAG_OFFSET: u64 = 80;

/// The offset in an open record of its event queue, and of the queue's
/// count [OFW-4.93 lv1 a:0x2595F8] [OFW-4.93 lv1 a:0x259624-0x259640].
const RECORD_QUEUE_OFFSET: u64 = 1392;
const RECORD_QUEUE_COUNT_OFFSET: u64 = 1392 + 16;

/// The offset in the bus object of the device-ready count
/// [OFW-4.93 lv1 a:0x259534].
const DEVICE_READY_COUNT_OFFSET: u64 = 3792;

/// The offset in the bus object of the table, and the bytes of each entry
/// [OFW-4.93 lv1 a:0x259578-0x25958C].
const TABLE_OFFSET: u64 = 3592;
const TABLE_ENTRY_BYTES: u64 = 32;

/// The `sc` of the kernel's `open_device` of the storage bus
/// [OFW-4.93 lv2_kernel a:0x80000000000AE110].
const KERNEL_OPEN_SC: u64 = 0x8000_0000_000A_E110;

/// The `sc` of the kernel's notification write
/// [OFW-4.93 lv2_kernel a:0x80000000000AE250].
const KERNEL_WRITE_SC: u64 = 0x8000_0000_000A_E250;

/// The `sc` of the kernel's notification read
/// [OFW-4.93 lv2_kernel a:0x80000000000AE408].
const KERNEL_READ_SC: u64 = 0x8000_0000_000A_E408;

/// The addresses of the write's and the read's buffers, in the real mode
/// region.
const WRITE_BUFFER: u64 = 0x1_0000;
const READ_BUFFER: u64 = 0x1_1000;

/// The event mask that the kernel stores at +8 of its write's buffer
/// [OFW-4.93 lv2_kernel a:0x80000000000AE1F4-0x80000000000AE200].
const KERNEL_MASK: u64 = 2;

/// The bound on the kernel's reads in the test, above the table's five
/// devices.
const MAX_READS: usize = 16;

/// The doubleword at `address` in the file `name` of the peek directory
/// `dir`. Panics unless both reads of the doubleword agree.
fn captured(dir: &str, name: &str, address: u64) -> u64 {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(dir)
        .join(name);
    let file = path.display().to_string();
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{file}: {e}"));
    let mut lines = text.lines();
    assert_eq!(
        lines.next(),
        Some("address\treads\tagree\tdoubleword"),
        "{file}: header"
    );
    let hex = |cell: &str| {
        let digits = cell
            .strip_prefix("0x")
            .unwrap_or_else(|| panic!("{file}: {cell} is not 0x hex"));
        u64::from_str_radix(digits, 16).unwrap_or_else(|e| panic!("{file}: {cell}: {e}"))
    };
    let row: Vec<&str> = lines
        .map(|line| line.split('\t').collect::<Vec<_>>())
        .find(|row| row.first().map(|cell| hex(cell)) == Some(address))
        .unwrap_or_else(|| panic!("{file}: no row for {address:#x}"));
    assert_eq!(row.len(), 4, "{file}: {address:#x} columns");
    assert_eq!(row[1..3], ["2", "yes"], "{file}: {address:#x}");
    hex(row[3])
}

fn registration_word(name: &str, address: u64) -> u64 {
    captured(REGISTRATION_CAPTURE, name, address)
}

fn request_word(name: &str, address: u64) -> u64 {
    captured(
        &format!("{NOTIFY_REQUEST_CAPTURE}/peek/cech20-cex-493"),
        name,
        address,
    )
}

/// The console's open record of the game OS partition: the last record
/// of the list in both captures, which holds the partition of slot 1 and
/// the kernel's event mask.
fn console_game_os_record() -> u64 {
    let bus = registration_word("bus.tsv", STORAGE_BUS_GLOBAL);
    assert_eq!(request_word("bus.tsv", STORAGE_BUS_GLOBAL), bus);
    let record = registration_word("list.tsv", bus + LAST_RECORD_OFFSET);
    assert_eq!(request_word("list.tsv", bus + LAST_RECORD_OFFSET), record);
    let game_os = request_word("slots.tsv", bus + SLOT_1_OFFSET);
    assert_eq!(
        request_word("records.tsv", record + RECORD_PARTITION_OFFSET),
        game_os
    );
    assert_eq!(
        request_word("records.tsv", record + RECORD_MASK_OFFSET),
        KERNEL_MASK
    );
    record
}

fn kernel_call(number: u64, pc: u64, args: &[u64]) -> HypercallRequest {
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

fn answered(
    hv: &mut Hypervisor,
    request: &HypercallRequest,
    memory: &GuestMemory,
) -> HypercallAnswer {
    match hv.dispatch_with(request, memory) {
        HypercallDisposition::Answered(answer) => answer,
        HypercallDisposition::Refused(refusal) => panic!("{refusal:?}"),
    }
}

/// The kernel's memory: the write's buffer with the command 0 and
/// [`KERNEL_MASK`].
fn kernel_memory() -> GuestMemory {
    let mut memory = GuestMemory::new(0x2_0000);
    let head = ByteRange::new(GuestAddr::new(WRITE_BUFFER), 16).expect("the buffer head");
    let mut bytes = NOTIFY_WRITE_COMMAND.to_be_bytes().to_vec();
    bytes.extend_from_slice(&KERNEL_MASK.to_be_bytes());
    memory.apply_commit(head, &bytes).expect("the buffer write");
    memory
}

/// The kernel's open, its notification write, and then a status call
/// and a read for each event, until a read waits
/// [OFW-4.93 lv2_kernel a:0x80000000000AE100-0x80000000000AE408].
#[test]
fn the_kernel_reads_get_the_captured_reports_and_wait_under_the_captured_tag() {
    let record = console_game_os_record();
    let bus = registration_word("bus.tsv", STORAGE_BUS_GLOBAL);
    let memory = kernel_memory();
    let mut hv = Hypervisor::new();
    hv.set_firmware("4.93");
    let open = kernel_call(OPEN_DEVICE, KERNEL_OPEN_SC, &[4, u64::MAX, 0]);
    assert_eq!(answered(&mut hv, &open, &memory).r3, 0, "open");
    let write = kernel_call(
        STORAGE_WRITE,
        KERNEL_WRITE_SC,
        &[u64::MAX, 0, 0, 1, 0, WRITE_BUFFER],
    );
    assert_eq!(answered(&mut hv, &write, &memory).r3, 0, "write");
    let status = kernel_call(STORAGE_GET_ASYNC_STATUS, KERNEL_WRITE_SC, &[u64::MAX]);
    let read = kernel_call(
        STORAGE_READ,
        KERNEL_READ_SC,
        &[u64::MAX, 0, 0, 1, 0, READ_BUFFER],
    );
    let mut reports = Vec::new();
    for _ in 0..MAX_READS {
        assert_eq!(answered(&mut hv, &status, &memory).r3, 0, "status");
        let answer = answered(&mut hv, &read, &memory);
        assert_eq!(answer.r3, 0);
        let Some(Effect::SharedWriteIntent { bytes, .. }) = answer.effects.first() else {
            break;
        };
        reports.push(bytes.bytes().to_vec());
    }
    let reported = registration_word("records.tsv", record + RECORD_REPORT_INDEX_OFFSET);
    let ready = registration_word("counts.tsv", bus + DEVICE_READY_COUNT_OFFSET);
    assert_eq!(reports.len() as u64, reported);
    assert_eq!(reported, ready);
    for (index, report) in (0u64..).zip(&reports) {
        let entry = bus + TABLE_OFFSET + index * TABLE_ENTRY_BYTES;
        let expected: Vec<u8> = std::iter::once(NOTIFY_READ_DEVICE_EVENT)
            .chain((0..4).map(|word| registration_word("table.tsv", entry + word * 8)))
            .flat_map(u64::to_be_bytes)
            .collect();
        assert_eq!(*report, expected, "report {index}");
    }
    let storage = hv.groups().storage();
    assert_eq!(storage.reported_devices(), reported);
    let waiting = storage.waiting_read().expect("a waiting read");
    assert_eq!(
        u64::from(waiting.tag),
        registration_word("records.tsv", record + RECORD_WAITING_TAG_OFFSET)
    );
    assert_ne!(
        registration_word("records.tsv", record + RECORD_WAITING_BUFFER_OFFSET),
        0
    );
    assert_eq!(
        registration_word("records.tsv", record + RECORD_TYPE0_INDEX_OFFSET),
        0
    );
    assert_eq!(
        registration_word("records.tsv", record + RECORD_QUEUE_OFFSET),
        0
    );
    assert_eq!(
        registration_word("records.tsv", record + RECORD_QUEUE_COUNT_OFFSET),
        0
    );
}
