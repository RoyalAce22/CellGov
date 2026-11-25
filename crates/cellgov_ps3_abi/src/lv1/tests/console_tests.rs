//! The console destruct worker and the gate, held to the live table
//! capture and the gate capture.

use super::*;
use crate::lv1::call::{lookup, LOGICAL_CONSOLE_DESTRUCT};
use crate::lv1::table::{table_entry, TableEntry, LIVE_TABLE_CAPTURE};

/// The memory call thunk, the live 4.93 table entry of each memory call.
const MEMORY_CALL_THUNK: u32 = 0x2C89CC;

/// The rows of the capture file `file` below `capture`, after its header
/// `header`, split at tabs.
fn capture_rows(capture: &str, file: &str, header: &str) -> Vec<Vec<String>> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(capture)
        .join(file);
    let name = path.display().to_string();
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{name}: {e}"));
    let mut lines = text.lines();
    assert_eq!(lines.next(), Some(header), "{name}: header");
    lines
        .map(|line| line.split('\t').map(str::to_string).collect())
        .collect()
}

fn hex(cell: &str) -> u64 {
    u64::from_str_radix(cell.trim_start_matches("0x"), 16).unwrap_or_else(|e| panic!("{cell}: {e}"))
}

#[test]
fn the_4_93_destruct_descriptor_is_the_captured_memory_call_entry_of_106() {
    let worker = console_destruct_worker("4.93").expect("the 4.93 worker");
    assert_eq!(
        table_entry("4.93", LOGICAL_CONSOLE_DESTRUCT),
        Some(TableEntry::Handler(MEMORY_CALL_THUNK))
    );
    let rows = capture_rows(
        LIVE_TABLE_CAPTURE,
        "peek/cech20-cex-493/memory_call_table.tsv",
        "number\tdescriptor",
    );
    let row = rows
        .iter()
        .find(|row| row[0] == LOGICAL_CONSOLE_DESTRUCT.to_string())
        .expect("a row for 106");
    assert_eq!(hex(&row[1]), u64::from(worker.descriptor));
    assert_eq!(console_destruct_worker("4.92"), None);
}

#[test]
fn the_reference_console_holds_the_4_93_gate_off() {
    let worker = console_destruct_worker("4.93").expect("the 4.93 worker");
    let rows = capture_rows(
        CONSOLE_GATE_CAPTURE,
        "peek/cech20-cex-493/gate.tsv",
        "address\treads\tagree\tbyte",
    );
    assert_eq!(rows.len(), 1);
    let row = &rows[0];
    assert_eq!(hex(&row[0]), u64::from(worker.gate));
    assert_eq!((row[1].as_str(), row[2].as_str()), ("2", "yes"));
    assert_eq!(hex(&row[3]), u64::from(CONSOLE_GATE_OFF));
    assert!(console_gate_off("4.93"));
    assert!(!console_gate_off("4.92"));
    // Each gate-off firmware needs a capture read here.
    assert_eq!(CONSOLE_GATE_OFF_FIRMWARE, ["4.93"]);
}

#[test]
fn each_firmware_with_the_gate_off_has_its_worker_read() {
    assert!(!CONSOLE_GATE_OFF_FIRMWARE.is_empty());
    for firmware in CONSOLE_GATE_OFF_FIRMWARE {
        assert!(console_destruct_worker(firmware).is_some(), "{firmware}");
    }
}

/// The kernel's wrapper passes one input and reads no output; see
/// [`LOGICAL_CONSOLE_DESTRUCT`].
#[test]
fn the_destruct_row_takes_one_input_and_gives_no_output() {
    let row = lookup(LOGICAL_CONSOLE_DESTRUCT).expect("a row for 106");
    assert_eq!(
        (row.name, row.inputs, row.outputs),
        ("undocumented_function_106", 1, 0)
    );
}
