//! The 4.93 hypervisor's answer to the kernel's notification write, held
//! to the `lv1_storage_notify_request` capture of the storage bus's
//! requests on the reference console after boot.

use super::*;
use crate::group::interrupt::InterruptChange;
use crate::group::state::StateChange;
use crate::Hypervisor;
use cellgov_event::UnitId;
use cellgov_ps3_abi::lv1::call::{
    CONFIGURE_IRQ_STATE_BITMAP, CONNECT_INTERRUPT_EVENT_RECEIVE_PORT, CONNECT_IRQ_PLUG_EXT,
    CONSTRUCT_EVENT_RECEIVE_PORT, OPEN_DEVICE,
};
use cellgov_ps3_abi::lv1::ppe::game_os_logical_ppe_id;
use cellgov_ps3_abi::lv1::storage::NOTIFY_REQUEST_CAPTURE;
use cellgov_time::GuestTicks;

/// The global that holds the storage bus object, from the TOC slot at
/// r2 - 23408 [OFW-4.93 lv1 a:0x251340-0x251360]
/// [OFW-4.93 lv1 a:0x3544C8-0x3544CC].
const STORAGE_BUS_GLOBAL: u64 = 0x36_8388;

/// The offset in the bus object of the request bitmap
/// [OFW-4.93 lv1 a:0x253990].
const BITMAP_OFFSET: u64 = 2680;

/// The offset in the bus object of the entry of tag 0, and the bytes of
/// each entry [OFW-4.93 lv1 a:0x2539FC-0x253A18].
const ENTRY_OFFSET: u64 = 2688;
const ENTRY_BYTES: u64 = 48;

/// The offset in the bus object of slot 1's partition
/// [OFW-4.93 lv1 a:0x2547B0].
const SLOT_1_OFFSET: u64 = 2608;

/// The offset in the bus object of the list of open records, and of the
/// list's last record [OFW-4.93 lv1 a:0x259218-0x259244].
const LAST_RECORD_OFFSET: u64 = 3552 + 8;

/// The offset in the bus object of slot 1's port: +2584 of the slot
/// [OFW-4.93 lv1 a:0x2564FC-0x256504].
const SLOT_1_PORT_OFFSET: u64 = 2584 + 64;

/// The offset in an open record of its partition
/// [OFW-4.93 lv1 a:0x25A798].
const RECORD_PARTITION_OFFSET: u64 = 40;

/// The offset in an open record of its event mask
/// [OFW-4.93 lv1 a:0x25A8D0].
const RECORD_MASK_OFFSET: u64 = 48;

/// The `sc` of the kernel's `open_device` of the storage bus
/// [OFW-4.93 lv2_kernel a:0x80000000000AE110].
const KERNEL_OPEN_SC: u64 = 0x8000_0000_000A_E110;

/// The `sc` of the port setup's `construct_event_receive_port`
/// [OFW-4.93 lv2_kernel a:0x8000000000096494].
const KERNEL_PORT_SC: u64 = 0x8000_0000_0009_6494;

/// The `sc` of the port setup's `connect_interrupt_event_receive_port`
/// [OFW-4.93 lv2_kernel a:0x80000000000964CC].
const KERNEL_CONNECT_SC: u64 = 0x8000_0000_0009_64CC;

/// The `sc` of the kernel's `connect_irq_plug_ext`
/// [OFW-4.93 lv2_kernel a:0x80000000000087F8].
const KERNEL_PLUG_SC: u64 = 0x8000_0000_0000_87F8;

/// The `sc` of the kernel's notification write
/// [OFW-4.93 lv2_kernel a:0x80000000000AE250].
const KERNEL_SC: u64 = 0x8000_0000_000A_E250;

/// The address of the request's buffer, in the real mode region.
const BUFFER_ADDRESS: u64 = 0x1_0000;

/// An 8-byte aligned address for thread 0's interrupt state bitmap.
const THREAD_0_BITMAP: u64 = 0x0036_9000;

/// The bytes of the caller's real address space in this test.
const MEMORY_BYTES: usize = 0x0037_0000;

/// The plug that the test connects the port to.
const PLUG: u64 = 9;

/// The event mask that the kernel stores at +8 of its buffer
/// [OFW-4.93 lv2_kernel a:0x80000000000AE1F4-0x80000000000AE200].
const KERNEL_MASK: u64 = 2;

/// The value of the doubleword at `address` in the capture's file
/// `name`. Panics unless both reads of the doubleword agree.
fn captured(name: &str, address: u64) -> u64 {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(NOTIFY_REQUEST_CAPTURE)
        .join("peek/cech20-cex-493")
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

/// The tag of the request that the console's lv1 holds for the game OS
/// partition, the partition of slot 1.
fn console_game_os_tag() -> u64 {
    let bus = captured("bus.tsv", STORAGE_BUS_GLOBAL);
    let game_os = captured("slots.tsv", bus + SLOT_1_OFFSET);
    let bitmap = captured("request.tsv", bus + BITMAP_OFFSET);
    let tags: Vec<u64> = (0..u64::from(NOTIFY_REQUEST_TAGS))
        .filter(|&tag| bitmap & (1 << tag) != 0)
        .filter(|&tag| captured("entries.tsv", bus + ENTRY_OFFSET + tag * ENTRY_BYTES) == game_os)
        .collect();
    assert_eq!(tags.len(), 1, "the game OS partition's tags {tags:?}");
    tags[0]
}

/// The event mask of the console's open record of the game OS
/// partition, the last record of the list. The write method walks the
/// list from +3552 [OFW-4.93 lv1 a:0x25A780] and matches the partition
/// at +40 of each record [OFW-4.93 lv1 a:0x25A798].
fn console_game_os_mask() -> u64 {
    let bus = captured("bus.tsv", STORAGE_BUS_GLOBAL);
    let game_os = captured("slots.tsv", bus + SLOT_1_OFFSET);
    let last = captured("list.tsv", bus + LAST_RECORD_OFFSET);
    assert_eq!(
        captured("records.tsv", last + RECORD_PARTITION_OFFSET),
        game_os
    );
    captured("records.tsv", last + RECORD_MASK_OFFSET)
}

/// The references of the port in the console's slot 1. Bits 2 to 31 of
/// the word at +8 of an object count its references
/// [OFW-4.93 lv1 a:0x2BED58-0x2BED6C], and the completion function tests
/// them before it sends the event [OFW-4.93 lv1 a:0x256518-0x256524].
fn console_slot_1_port_references() -> u64 {
    let bus = captured("bus.tsv", STORAGE_BUS_GLOBAL);
    let port = captured("slots.tsv", bus + SLOT_1_PORT_OFFSET);
    (captured("port_ref.tsv", port + 8) >> 32) >> 2
}

fn kernel_call(number: u64, pc: u64, args: &[u64]) -> HypercallRequest {
    let mut inputs = [0; 8];
    inputs[..args.len()].copy_from_slice(args);
    HypercallRequest {
        unit: UnitId::new(0),
        number,
        inputs,
        pc,
        lr: 0,
        thread: Some(0),
        problem_state: false,
        tick: GuestTicks::new(33_990),
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

/// The kernel's memory: thread 0's bitmap, and the buffer with the
/// command 0 and [`KERNEL_MASK`]
/// [OFW-4.93 lv2_kernel a:0x80000000000AE1DC-0x80000000000AE204].
fn kernel_memory() -> GuestMemory {
    let mut memory = GuestMemory::new(MEMORY_BYTES);
    let head = ByteRange::new(GuestAddr::new(BUFFER_ADDRESS), 16).expect("the buffer head");
    let mut bytes = NOTIFY_WRITE_COMMAND.to_be_bytes().to_vec();
    bytes.extend_from_slice(&KERNEL_MASK.to_be_bytes());
    memory.apply_commit(head, &bytes).expect("the buffer write");
    memory
}

/// The kernel's storage bus steps on thread 0: the open, the port, its
/// connect to the bus and to a plug, then the notification write
/// [OFW-4.93 lv2_kernel a:0x80000000000AE100-0x80000000000AE250].
#[test]
fn the_kernel_notify_write_gets_the_console_tag_and_sends_an_event_on_the_storage_port() {
    assert_ne!(console_slot_1_port_references(), 0, "the port's references");
    let memory = kernel_memory();
    let mut hv = Hypervisor::new();
    hv.set_firmware("4.93");
    let ppe = game_os_logical_ppe_id("4.93").expect("the game OS PPE");
    let bitmap = kernel_call(
        CONFIGURE_IRQ_STATE_BITMAP,
        KERNEL_PLUG_SC,
        &[ppe, 0, THREAD_0_BITMAP],
    );
    assert_eq!(answered(&mut hv, &bitmap, &memory).r3, 0, "bitmap");
    let open = kernel_call(OPEN_DEVICE, KERNEL_OPEN_SC, &[4, u64::MAX, 0]);
    assert_eq!(answered(&mut hv, &open, &memory).r3, 0, "open_device");
    let made = answered(
        &mut hv,
        &kernel_call(CONSTRUCT_EVENT_RECEIVE_PORT, KERNEL_PORT_SC, &[]),
        &memory,
    );
    let port = made.outputs().expect("a table return")[0];
    let connect = kernel_call(
        CONNECT_INTERRUPT_EVENT_RECEIVE_PORT,
        KERNEL_CONNECT_SC,
        &[4, u64::MAX, port, 0],
    );
    assert_eq!(answered(&mut hv, &connect, &memory).r3, 0, "connect");
    let plug = kernel_call(
        CONNECT_IRQ_PLUG_EXT,
        KERNEL_PLUG_SC,
        &[ppe, 0, PLUG, port, 0],
    );
    assert_eq!(answered(&mut hv, &plug, &memory).r3, 0, "plug");
    let write = kernel_call(
        STORAGE_WRITE,
        KERNEL_SC,
        &[u64::MAX, 0, 0, 1, 0, BUFFER_ADDRESS],
    );
    let answer = answered(&mut hv, &write, &memory);
    assert_eq!(answer.r3, 0);
    assert_eq!(
        answer.outputs(),
        Some([console_game_os_tag(), 0, 0, 0, 0, 0, 0])
    );
    assert!(
        answer
            .changes
            .contains(&StateChange::from(InterruptChange::EventPending(port))),
        "{:?}",
        answer.changes
    );
    assert!(hv.groups().interrupt().ports().is_pending(port));
    assert_eq!(
        hv.groups().storage().notify_event_mask(),
        Some(console_game_os_mask())
    );
}
