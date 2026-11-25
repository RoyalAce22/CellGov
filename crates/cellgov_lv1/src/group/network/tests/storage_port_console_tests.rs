//! The 4.93 hypervisor's answer to the kernel's
//! `connect_interrupt_event_receive_port` for the storage bus, held to the
//! `lv1_storage_port_id` capture of the port that lv1 connected to the
//! storage bus on the reference console after boot.

use super::*;
use crate::Hypervisor;
use cellgov_event::UnitId;
use cellgov_ps3_abi::lv1::call::{
    CONNECT_INTERRUPT_EVENT_RECEIVE_PORT, CONSTRUCT_EVENT_RECEIVE_PORT, OPEN_DEVICE,
};
use cellgov_ps3_abi::lv1::event_port::STORAGE_PORT_CAPTURE;
use cellgov_ps3_abi::lv1::network::STORAGE_PORT_ID_LIMIT;
use cellgov_time::GuestTicks;

/// The global that holds the storage bus object, from the TOC slot at
/// r2 - 26656 [OFW-4.93 lv1 a:0x353818-0x35381C].
const STORAGE_BUS_GLOBAL: u64 = 0x36_8388;

/// The offset in the storage bus object of the port that slot 1's handle
/// holds: +16 of the handle at 2568 + 64
/// [OFW-4.93 lv1 a:0x25554C-0x255570] [OFW-4.93 lv1 a:0x2BEDD4].
const SLOT_1_PORT_OFFSET: u64 = 2568 + 64 + 16;

/// The offset in an object of its id in the partition's object table
/// [OFW-4.93 lv1 a:0x2CC6DC-0x2CC6EC].
const ID_OFFSET: u64 = 72;

/// The offset in a port of its partition
/// [OFW-4.93 lv1 a:0x2DF670].
const PARTITION_OFFSET: u64 = 120;

/// The offset in a partition of its object table
/// [OFW-4.93 lv1 a:0x245FFC-0x246008].
const TABLE_OFFSET: u64 = 88;

/// The `sc` of the kernel's `open_device` of the storage bus
/// [OFW-4.93 lv2_kernel a:0x80000000000AE110].
const KERNEL_OPEN_SC: u64 = 0x8000_0000_000A_E110;

/// The `sc` of the port setup's `construct_event_receive_port`
/// [OFW-4.93 lv2_kernel a:0x8000000000096494].
const KERNEL_PORT_SC: u64 = 0x8000_0000_0009_6494;

/// The `sc` of the port setup's `connect_interrupt_event_receive_port`
/// [OFW-4.93 lv2_kernel a:0x80000000000964CC].
const KERNEL_SC: u64 = 0x8000_0000_0009_64CC;

/// The value of the doubleword at `address` in the capture's file
/// `name`. Panics unless both reads of the doubleword agree.
fn captured(name: &str, address: u64) -> u64 {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(STORAGE_PORT_CAPTURE)
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

/// The port that slot 1's handle of the console's storage bus object
/// holds.
fn console_slot_1_port() -> u64 {
    let bus = captured("bus.tsv", STORAGE_BUS_GLOBAL);
    captured("bus_port.tsv", bus + SLOT_1_PORT_OFFSET)
}

/// The kernel's call `number` from the `sc` at `pc` with r3 to r6.
fn kernel_call(number: u64, pc: u64, r3_to_r6: [u64; 4]) -> HypercallRequest {
    let mut inputs = [0; 8];
    inputs[..4].copy_from_slice(&r3_to_r6);
    HypercallRequest {
        unit: UnitId::new(0),
        number,
        inputs,
        pc,
        lr: 0,
        thread: Some(0),
        problem_state: false,
        tick: GuestTicks::new(0),
    }
}

/// The answer of `request`. Panics when the hypervisor refuses it.
fn answer(hv: &mut Hypervisor, request: &HypercallRequest) -> HypercallAnswer {
    match hv.dispatch(request) {
        HypercallDisposition::Answered(answer) => answer,
        HypercallDisposition::Refused(refusal) => panic!("{refusal:?}"),
    }
}

/// The kernel opens the storage bus, makes the port with hypercall 18 and
/// gives that id to hypercall 197
/// [OFW-4.93 lv2_kernel a:0x80000000000AE100-0x80000000000AE118]
/// [OFW-4.93 lv2_kernel a:0x8000000000096490-0x80000000000964CC].
#[test]
fn the_kernel_call_connects_the_port_that_the_console_storage_bus_slot_holds() {
    let mut hv = Hypervisor::new();
    hv.set_firmware("4.93");
    let open = kernel_call(OPEN_DEVICE, KERNEL_OPEN_SC, [4, u64::MAX, 0, 0]);
    assert_eq!(answer(&mut hv, &open).r3, 0, "open_device");
    let made = answer(
        &mut hv,
        &kernel_call(CONSTRUCT_EVENT_RECEIVE_PORT, KERNEL_PORT_SC, [0; 4]),
    );
    assert_eq!(made.r3, 0, "construct_event_receive_port");
    let port = made.outputs().expect("a table return")[0];
    let connect = kernel_call(
        CONNECT_INTERRUPT_EVENT_RECEIVE_PORT,
        KERNEL_SC,
        [4, u64::MAX, port, 0],
    );
    let connected = answer(&mut hv, &connect);
    assert_eq!(connected.r3, 0, "connect_interrupt_event_receive_port");
    assert_eq!(connected.outputs(), Some([0; 7]), "r4 to r10");
    let console_port = console_slot_1_port();
    assert_eq!(
        hv.groups().device_bus().storage_bus_port(),
        Some(captured("port.tsv", console_port + ID_OFFSET))
    );
}

/// The segment shape is the word at +32 of the table
/// [OFW-4.93 lv1 a:0x255348-0x255370].
#[test]
fn the_segment_0_limit_is_the_captured_shape_of_the_partition_table() {
    let port = console_slot_1_port();
    let table = captured("port.tsv", port + PARTITION_OFFSET) + TABLE_OFFSET;
    let shape = captured("table.tsv", table + 32);
    assert_eq!(1 << (shape >> 32), STORAGE_PORT_ID_LIMIT);
    let id = captured("port.tsv", port + ID_OFFSET);
    assert!(id < STORAGE_PORT_ID_LIMIT, "id {id} is in segment 0");
}
