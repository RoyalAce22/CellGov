//! The port of the 4.93 kernel's storage bus interrupt, after the port of
//! its network device interrupt, held to the console captures of both
//! ports' ids.

use super::*;
use crate::Hypervisor;
use cellgov_event::UnitId;
use cellgov_ps3_abi::lv1::call::CONSTRUCT_EVENT_RECEIVE_PORT;
use cellgov_ps3_abi::lv1::event_port::{NETWORK_PORT_CAPTURE, STORAGE_PORT_CAPTURE};
use cellgov_time::GuestTicks;
use std::path::Path;

/// The `sc` of the kernel's call that makes the network device's
/// interrupt port [OFW-4.93 lv2_kernel a:0x80000000000a28e0].
const NETWORK_SC: u64 = 0x8000_0000_000a_28e0;

/// The `sc` of the kernel's port setup of an interrupt object of type 2,
/// which the storage bus function reaches first
/// [OFW-4.93 lv2_kernel a:0x8000000000096494].
const STORAGE_SC: u64 = 0x8000_0000_0009_6494;

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

/// The path and the text of the file `name` of the console capture
/// `capture`. Panics unless the first line is `header`.
fn capture_file(capture: &str, name: &str, header: &str) -> (String, String) {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(capture)
        .join("peek/cech20-cex-493")
        .join(name);
    let file = path.display().to_string();
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{file}: {e}"));
    assert_eq!(text.lines().next(), Some(header), "{file}: header");
    (file, text)
}

/// The value of the `0x` hex cell `cell` of the file `file`.
fn hex(file: &str, cell: &str) -> u64 {
    let digits = cell
        .strip_prefix("0x")
        .unwrap_or_else(|| panic!("{file}: {cell} is not 0x hex"));
    u64::from_str_radix(digits, 16).unwrap_or_else(|e| panic!("{file}: {cell}: {e}"))
}

/// The address and the value of the row `field` of the network capture.
/// Panics unless both reads of the doubleword agree.
fn network_captured(field: &str) -> (u64, u64) {
    let (file, text) = capture_file(
        NETWORK_PORT_CAPTURE,
        "network_port.tsv",
        "address\tfield\twidth\treads\tagree\tvalue",
    );
    let row: Vec<&str> = text
        .lines()
        .skip(1)
        .map(|line| line.split('\t').collect::<Vec<_>>())
        .find(|row| row.get(1) == Some(&field))
        .unwrap_or_else(|| panic!("{file}: no row for {field}"));
    assert_eq!(row.len(), 6, "{file}: {field} columns");
    assert_eq!(row[2..5], ["8", "2", "yes"], "{file}: {field}");
    (hex(&file, row[0]), hex(&file, row[5]))
}

/// The value of the doubleword at `address` in the storage capture's file
/// `name`. Panics unless both reads of the doubleword agree.
fn storage_captured(name: &str, address: u64) -> u64 {
    let (file, text) = capture_file(
        STORAGE_PORT_CAPTURE,
        name,
        "address\treads\tagree\tdoubleword",
    );
    let row: Vec<&str> = text
        .lines()
        .skip(1)
        .map(|line| line.split('\t').collect::<Vec<_>>())
        .find(|row| row.first().map(|cell| hex(&file, cell)) == Some(address))
        .unwrap_or_else(|| panic!("{file}: no row for {address:#x}"));
    assert_eq!(row.len(), 4, "{file}: {address:#x} columns");
    assert_eq!(row[1..3], ["2", "yes"], "{file}: {address:#x}");
    hex(&file, row[3])
}

/// The id of the port that lv1 connected to the network device's
/// interrupt on the reference console.
fn network_port() -> u64 {
    let (_, port) = network_captured("port");
    let (address, id) = network_captured("port_id");
    assert_eq!(address, port + ID_OFFSET, "the network port's id field");
    id
}

/// The id of the port that slot 1's handle of the storage bus object
/// holds on the reference console.
fn storage_port() -> u64 {
    let bus = storage_captured("bus.tsv", STORAGE_BUS_GLOBAL);
    let port = storage_captured("bus_port.tsv", bus + SLOT_1_PORT_OFFSET);
    storage_captured("port.tsv", port + ID_OFFSET)
}

fn construct(thread: u8, pc: u64) -> HypercallRequest {
    HypercallRequest {
        unit: UnitId::new(0),
        number: CONSTRUCT_EVENT_RECEIVE_PORT,
        inputs: [0xA5A5_A5A5_A5A5_A5A5; 8],
        pc,
        lr: 0,
        thread: Some(thread),
        problem_state: false,
        tick: GuestTicks::new(33_914),
    }
}

fn constructed(id: u64) -> HypercallDisposition {
    let mut answer = HypercallAnswer::lv1_status(0, [id, 0, 0, 0, 0, 0, 0]);
    answer.push_change(InterruptChange::PortConstructed(EventReceivePort::new(id)));
    HypercallDisposition::Answered(answer)
}

fn refused(r: &HypercallRequest, what: &'static str) -> HypercallDisposition {
    HypercallDisposition::Refused(HypercallRefusal::UnmodelledInput {
        number: CONSTRUCT_EVENT_RECEIVE_PORT,
        unit: r.unit,
        pc: r.pc,
        what,
    })
}

/// A 4.93 hypervisor after the network device's call.
fn after_network_call() -> Hypervisor {
    let mut hv = Hypervisor::new();
    hv.set_firmware("4.93");
    assert_eq!(
        hv.dispatch(&construct(0, NETWORK_SC)),
        constructed(network_port())
    );
    hv
}

#[test]
fn the_storage_call_gets_the_captured_id_with_r5_to_r10_cleared_and_is_kept() {
    let mut hv = after_network_call();
    let before = hv.sync_term();
    let storage = storage_port();
    assert_eq!(hv.dispatch(&construct(0, STORAGE_SC)), constructed(storage));
    let ids: Vec<u64> = hv.event_receive_ports().ports().keys().copied().collect();
    assert_eq!(ids, [network_port(), storage]);
    assert_eq!(
        hv.event_receive_ports().port(storage),
        Some(&EventReceivePort::new(storage))
    );
    assert_ne!(hv.sync_term(), before);
    assert_eq!(hv.refusal(), None);
}

#[test]
fn the_storage_sc_on_thread_1_is_refused() {
    let mut hv = after_network_call();
    let r = construct(1, STORAGE_SC);
    assert_eq!(
        hv.dispatch(&r),
        refused(
            &r,
            "an event receive port call whose port id no source gives for its thread and address"
        )
    );
    let ids: Vec<u64> = hv.event_receive_ports().ports().keys().copied().collect();
    assert_eq!(ids, [network_port()]);
}
