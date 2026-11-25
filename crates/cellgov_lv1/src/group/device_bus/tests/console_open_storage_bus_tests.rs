//! The hypervisor holds the storage bus open after the 4.93 kernel's
//! open, as lv1 does on the reference console. The committed capture
//! `tests/micro/lv1_storage_bus_open` gives the console's state.

use super::*;
use crate::Hypervisor;
use cellgov_event::UnitId;
use cellgov_ps3_abi::lv1::device_bus::STORAGE_BUS_OPEN_CAPTURE;
use cellgov_ps3_abi::lv1::partition::LPAR_ID_GAME_OS;
use cellgov_time::GuestTicks;

/// The global that holds the storage bus object, from the TOC slot at
/// r2 - 26656 [OFW-4.93 lv1 a:0x353818-0x35381C].
const STORAGE_BUS_GLOBAL: u64 = 0x36_8388;

/// The method table that the object's constructor stores at +0
/// [OFW-4.93 lv1 a:0x259298] [OFW-4.93 lv1 a:0x354670-0x354674].
const METHOD_TABLE: u64 = 0x35_DD30;

/// The byte that the partition check tests, which the constructor sets
/// to 1 [OFW-4.93 lv1 a:0x25929C-0x2592A4] [OFW-4.93 lv1 a:0x2545DC].
const GATE: u64 = 3496;

/// The two partition slots, 64 bytes apart: the partition at +0, the r5
/// at +8 and the used byte at +16 [OFW-4.93 lv1 a:0x254710-0x25472C].
const SLOTS: [u64; 2] = [2544, 2608];

/// The record list: its last record at +8 of the list
/// [OFW-4.93 lv1 a:0x259204] [OFW-4.93 lv1 a:0x259244].
const RECORD_LIST: u64 = 3552;

/// The record's list at +32 and partition at +40
/// [OFW-4.93 lv1 a:0x259208] [OFW-4.93 lv1 a:0x25924C].
const RECORD_OWNER: u64 = 32;
const RECORD_PARTITION: u64 = 40;

/// The address and value of each row of the capture's file `name`, whose
/// value column is `column`. Each row must hold two reads that agree.
fn captured(name: &str, column: &str) -> Vec<(u64, u64)> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(STORAGE_BUS_OPEN_CAPTURE)
        .join("peek/cech20-cex-493")
        .join(name);
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let mut lines = text.lines();
    assert_eq!(
        lines.next(),
        Some(format!("address\treads\tagree\t{column}").as_str()),
        "{}",
        path.display()
    );
    lines
        .map(|line| {
            let cells: Vec<&str> = line.split('\t').collect();
            assert_eq!(cells.len(), 4, "{name}: {line}");
            assert_eq!(cells[1..3], ["2", "yes"], "{name}: {line}");
            (hex(cells[0]), hex(cells[3]))
        })
        .collect()
}

fn hex(cell: &str) -> u64 {
    let digits = cell
        .strip_prefix("0x")
        .unwrap_or_else(|| panic!("{cell} is not 0x hex"));
    u64::from_str_radix(digits, 16).unwrap_or_else(|e| panic!("{cell}: {e}"))
}

/// The storage bus object, after a check that it holds the constructor's
/// method table.
fn storage_bus_object() -> u64 {
    let walk = captured("walk.tsv", "doubleword");
    let [(global, object), (object_at, table)] = walk[..] else {
        panic!("walk.tsv holds 2 rows: {walk:?}");
    };
    assert_eq!(global, STORAGE_BUS_GLOBAL, "the global");
    assert_eq!(object_at, object, "the object's +0");
    assert_eq!(table, METHOD_TABLE, "the object's method table");
    object
}

/// The r5 that the game OS partition's slot keeps, after a check of the
/// slot's used byte.
fn game_os_slot_r5(object: u64) -> u64 {
    let slots = captured("slots.tsv", "doubleword");
    let used = captured("slot_used.tsv", "byte");
    assert_eq!(slots.len(), 4, "{slots:?}");
    assert_eq!(used.len(), 2, "{used:?}");
    let mut found = Vec::new();
    for (index, base) in SLOTS.into_iter().enumerate() {
        let (partition_at, partition) = slots[2 * index];
        let (r5_at, r5) = slots[2 * index + 1];
        let (used_at, used) = used[index];
        assert_eq!(partition_at, object + base, "slot {index} partition");
        assert_eq!(r5_at, object + base + 8, "slot {index} r5");
        assert_eq!(used_at, object + base + 16, "slot {index} used byte");
        if used == 1 && partition == LPAR_ID_GAME_OS {
            found.push(r5);
        }
    }
    assert_eq!(found.len(), 1, "the game OS partition holds one slot");
    found[0]
}

#[test]
fn the_capture_shows_the_checks_passed_for_the_game_os_partition() {
    let object = storage_bus_object();
    let gate = captured("gate.tsv", "byte");
    assert_eq!(gate, [(object + GATE, 1)], "the gate byte");
    let record = captured("record.tsv", "doubleword");
    let [(last_at, last), (owner_at, owner), (partition_at, partition)] = record[..] else {
        panic!("record.tsv holds 3 rows: {record:?}");
    };
    assert_eq!(last_at, object + RECORD_LIST + 8, "the list's last record");
    assert_eq!(owner_at, last + RECORD_OWNER, "the record's list");
    assert_eq!(owner, object + RECORD_LIST, "the record is in the list");
    assert_eq!(
        partition_at,
        last + RECORD_PARTITION,
        "the record's partition"
    );
    assert_eq!(partition, LPAR_ID_GAME_OS, "the record's partition");
}

/// The kernel opens the storage bus with r3 = 4, r4 = -1 and r5 = 0
/// [OFW-4.93 lv2_kernel a:0x80000000000ae100-0x80000000000ae110].
#[test]
fn the_kernel_open_leaves_the_storage_bus_open_with_the_captured_r5() {
    let r5 = game_os_slot_r5(storage_bus_object());
    let mut inputs = [0; 8];
    inputs[..3].copy_from_slice(&[4, u64::MAX, 0]);
    let mut hv = Hypervisor::new();
    hv.set_firmware("4.93");
    let HypercallDisposition::Answered(answer) = hv.dispatch(&HypercallRequest {
        unit: UnitId::new(0),
        number: OPEN_DEVICE,
        inputs,
        pc: 0x8000_0000_000A_E110,
        lr: 0,
        thread: Some(0),
        problem_state: false,
        tick: GuestTicks::ZERO,
    }) else {
        panic!("the kernel's storage bus open is refused");
    };
    assert_eq!(answer.r3, 0, "r3");
    assert_eq!(
        hv.groups().device_bus().storage_bus_open(),
        Some(StorageBusOpen { r5 }),
        "the slot's r5"
    );
}
