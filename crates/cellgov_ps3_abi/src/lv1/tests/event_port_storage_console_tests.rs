//! The 4.93 port id of the kernel's call that makes the storage bus's
//! interrupt port, held to the console capture of that port in lv1.

use super::*;

/// The global that holds the storage bus object, from the TOC slot at
/// r2 - 26656 [OFW-4.93 lv1 a:0x353818-0x35381C].
const STORAGE_BUS_GLOBAL: u64 = 0x36_8388;

/// The offset in the storage bus object of the port that slot 1
/// connects: +16 of the handle at +2632
/// [OFW-4.93 lv1 a:0x255520-0x25552C] [OFW-4.93 lv1 a:0x2BEDD4].
const BUS_PORT_OFFSET: u64 = 2648;

/// The offset in an object of its id in the partition's object table
/// [OFW-4.93 lv1 a:0x2CC6DC-0x2CC6EC].
const ID_OFFSET: u64 = 72;

/// The class that the storage connect checks, from the TOC slot at
/// r2 - 23160 [OFW-4.93 lv1 a:0x2553C0-0x2553CC] [OFW-4.93 lv1 a:0x3545C0].
const PORT_CLASS: u64 = 0x32_9310;

/// The word that `construct_event_receive_port` gives the port
/// constructor for +136, from the TOC slot r2 - 7896
/// [OFW-4.93 lv1 a:0x358160].
const PORT_WORD: u64 = 0x32_98d2;

/// The offset in a partition of its object table
/// [OFW-4.93 lv1 a:0x245FFC-0x246008].
const TABLE_OFFSET: u64 = 88;

/// The value of the doubleword at `address` in the capture's file
/// `name`. Both reads of the doubleword must agree.
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

/// The port at +2648 of the storage bus object.
fn bus_port() -> u64 {
    let bus = captured("bus.tsv", STORAGE_BUS_GLOBAL);
    captured("bus_port.tsv", bus + BUS_PORT_OFFSET)
}

#[test]
fn the_storage_call_gets_the_id_of_the_port_lv1_connects_to_the_storage_bus() {
    let port = bus_port();
    assert_eq!(captured("port.tsv", port + 48), PORT_CLASS, "class");
    assert_eq!(captured("port.tsv", port + 136), PORT_WORD, "+136");
    let id = captured("port.tsv", port + ID_OFFSET);
    assert_eq!(game_os_port_id("4.93", 0, STORAGE_SC), Some(id));
}

#[test]
fn the_table_slots_of_the_storage_and_network_ids_hold_their_captured_ports() {
    let port = bus_port();
    let id = game_os_port_id("4.93", 0, STORAGE_SC).expect("the storage call's port id");
    let network = game_os_port_id("4.93", 0, NETWORK_SC).expect("the network call's port id");
    let partition = captured("port.tsv", port + 56);
    assert_eq!(captured("port.tsv", port + 120), partition, "+120");
    let table = partition + TABLE_OFFSET;
    let segments = captured("table.tsv", table + 24);
    let shape = captured("table.tsv", table + 32);
    assert!(id < 1 << (shape >> 32), "id {id} is in segment 0");
    let segment = captured("table.tsv", segments);
    let stride = captured("table.tsv", segment + 48);
    let base = captured("table.tsv", segment + 72);
    assert_eq!(captured("table.tsv", base + id * stride), port, "slot {id}");
    assert_eq!(
        captured("table.tsv", base + network * stride),
        network_capture_port(),
        "slot {network}"
    );
}

/// The port that the network capture reads at +32 of the network
/// device's connection object.
fn network_capture_port() -> u64 {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(NETWORK_PORT_CAPTURE)
        .join("peek/cech20-cex-493/network_port.tsv");
    let file = path.display().to_string();
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{file}: {e}"));
    let row: Vec<&str> = text
        .lines()
        .map(|line| line.split('\t').collect::<Vec<_>>())
        .find(|row| row.get(1) == Some(&"port"))
        .unwrap_or_else(|| panic!("{file}: no port row"));
    assert_eq!(row.len(), 6, "{file}: port columns");
    assert_eq!(row[2..5], ["8", "2", "yes"], "{file}: port");
    let digits = row[5]
        .strip_prefix("0x")
        .unwrap_or_else(|| panic!("{file}: {} is not 0x hex", row[5]));
    u64::from_str_radix(digits, 16).unwrap_or_else(|e| panic!("{file}: {e}"))
}
