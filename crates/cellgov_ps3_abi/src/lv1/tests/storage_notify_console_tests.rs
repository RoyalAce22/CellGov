//! The lv1 state of the storage bus's notification requests, held to the
//! console capture of that state after boot.

use super::*;

/// The global that holds the storage bus object, from the TOC slot at
/// r2 - 23408 [OFW-4.93 lv1 a:0x251340-0x251360]
/// [OFW-4.93 lv1 a:0x3544C8-0x3544CC].
const STORAGE_BUS_GLOBAL: u64 = 0x36_8388;

/// The offset in the bus object of its state word
/// [OFW-4.93 lv1 a:0x256464].
const STATE_OFFSET: u64 = 3456;

/// The offset in the bus object of the doubleword that each request takes
/// [OFW-4.93 lv1 a:0x25A7CC].
const REQUEST_WORD_OFFSET: u64 = 3464;

/// The offset in the bus object of the request bitmap
/// [OFW-4.93 lv1 a:0x253990].
const BITMAP_OFFSET: u64 = 2680;

/// The offset in the bus object of the entry of tag 0, and the bytes of
/// each entry [OFW-4.93 lv1 a:0x2539FC-0x253A18].
const ENTRY_OFFSET: u64 = 2688;
const ENTRY_BYTES: u64 = 48;

/// The partition that the write method gives a path of its own
/// [OFW-4.93 lv1 a:0x25A71C-0x25A724].
const PARTITION_1: u64 = 1;

/// The value of the doubleword at `address` in the capture's file
/// `name`. Both reads of the doubleword must agree.
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

fn bus() -> u64 {
    captured("bus.tsv", STORAGE_BUS_GLOBAL)
}

/// The word at `offset` of the bus object: the high half of the
/// captured doubleword.
fn bus_word(offset: u64) -> u32 {
    (captured("request.tsv", bus() + offset) >> 32) as u32
}

#[test]
fn the_completing_state_is_the_captured_state_of_the_storage_bus() {
    assert_eq!(bus_word(STATE_OFFSET), STORAGE_BUS_COMPLETING_STATE);
}

#[test]
fn the_request_word_is_the_captured_word_at_3464() {
    assert_eq!(
        captured("request.tsv", bus() + REQUEST_WORD_OFFSET),
        NOTIFY_REQUEST_WORD
    );
}

#[test]
fn the_other_partition_tags_are_the_captured_tags_of_partition_1() {
    let bitmap = captured("request.tsv", bus() + BITMAP_OFFSET);
    assert_ne!(bitmap & u64::from(OTHER_PARTITION_NOTIFY_TAGS), 0);
    let mut partition_1 = 0u16;
    for tag in 0..NOTIFY_REQUEST_TAGS {
        if bitmap & (1 << tag) == 0 {
            continue;
        }
        let entry = bus() + ENTRY_OFFSET + u64::from(tag) * ENTRY_BYTES;
        if captured("entries.tsv", entry) == PARTITION_1 {
            partition_1 |= 1 << tag;
        }
    }
    assert_eq!(partition_1, OTHER_PARTITION_NOTIFY_TAGS);
}
