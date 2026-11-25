//! The 4.93 port ids of the kernel's calls that make the storage devices'
//! interrupt ports, held to the console capture of those ports in lv1.

use super::*;

/// The global that holds the storage bus object, from the TOC slot at
/// r2 - 26656 [OFW-4.93 lv1 a:0x353818-0x35381C].
const STORAGE_BUS_GLOBAL: u64 = 0x36_8388;

/// The offset in the storage bus object of the device object of device 0.
/// Device `id` is at this offset plus 8 x `id`
/// [OFW-4.93 lv1 a:0x2369AC-0x2369B4].
const DEVICES_OFFSET: u64 = 3816;

/// The storage devices of the 4.93 bus table, ids 0 to 4.
const DEVICES: u64 = 5;

/// The offset in a device object of slot 0's partition word. Slot 1's is
/// 64 bytes higher [OFW-4.93 lv1 a:0x2554F4] [OFW-4.93 lv1 a:0x255508].
const SLOT_PARTITION_OFFSET: u64 = 2544;

/// The offset in a device object of the port at +16 of slot 0's handle.
/// Slot 1's is 64 bytes higher
/// [OFW-4.93 lv1 a:0x25554C-0x255554] [OFW-4.93 lv1 a:0x255520-0x255524]
/// [OFW-4.93 lv1 a:0x2BEDD4].
const SLOT_PORT_OFFSET: u64 = 2568 + 16;

/// The offset in a partition of the word that the storage connect
/// compares with a slot's partition word [OFW-4.93 lv1 a:0x2554F8-0x255500].
const PARTITION_ID_OFFSET: u64 = 80;

/// The offset in an object of its id in the partition's object table
/// [OFW-4.93 lv1 a:0x2CC6DC-0x2CC6EC].
const ID_OFFSET: u64 = 72;

/// The class that the storage connect checks, from the TOC slot at
/// r2 - 23160 [OFW-4.93 lv1 a:0x2553C0-0x2553CC] [OFW-4.93 lv1 a:0x3545C0].
const PORT_CLASS: u64 = 0x32_9310;

/// The word that `construct_event_receive_port` gives the port
/// constructor for +136, from the TOC slot at r2 - 7896
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
        .join(STORAGE_DEVICE_PORT_CAPTURE)
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

/// The port that lv1 connects to device `device` for the partition with
/// the partition word `partition_id`.
fn device_port(device: u64, partition_id: u64) -> u64 {
    let bus = captured("bus.tsv", STORAGE_BUS_GLOBAL);
    let object = captured("devices.tsv", bus + DEVICES_OFFSET + 8 * device);
    let slot = (0..2)
        .find(|slot| {
            captured("slots.tsv", object + SLOT_PARTITION_OFFSET + 64 * slot) == partition_id
        })
        .unwrap_or_else(|| panic!("device {device}: no slot holds partition {partition_id}"));
    captured("device_port.tsv", object + SLOT_PORT_OFFSET + 64 * slot)
}

/// The game OS partition's port of each storage device, by device id. The
/// partition word 2 is the game OS partition's; see the capture of the
/// storage device opens.
fn game_os_ports() -> Vec<u64> {
    let ports: Vec<u64> = (0..DEVICES).map(|device| device_port(device, 2)).collect();
    for &port in &ports {
        let partition = captured("port.tsv", port + 56);
        assert_eq!(
            captured("port.tsv", port + 120),
            partition,
            "{port:#x} +120"
        );
        assert_eq!(
            captured("partition.tsv", partition + PARTITION_ID_OFFSET),
            2,
            "{port:#x}: the partition word"
        );
    }
    ports
}

#[test]
fn the_storage_calls_after_the_bus_call_get_the_ids_of_the_ports_lv1_connects_to_devices_0_to_4() {
    let ports = game_os_ports();
    assert_eq!(ports.len(), 5);
    let ids: Vec<u64> = ports
        .iter()
        .map(|&port| {
            assert_eq!(
                captured("port.tsv", port + 48),
                PORT_CLASS,
                "{port:#x} class"
            );
            assert_eq!(
                captured("port.tsv", port + 136),
                PORT_WORD,
                "{port:#x} +136"
            );
            captured("port.tsv", port + ID_OFFSET)
        })
        .collect();
    let rows: Vec<u64> = game_os_port_ids("4.93", 0, STORAGE_SC).collect();
    assert_eq!(rows.len(), 6);
    assert_eq!(rows[1..], ids[..]);
}

#[test]
fn the_table_slots_of_the_storage_device_ids_hold_their_captured_ports_and_are_in_use() {
    let ports = game_os_ports();
    let partition = captured("port.tsv", ports[0] + 56);
    let table = partition + TABLE_OFFSET;
    let segments = captured("table.tsv", table + 24);
    let shape = captured("table.tsv", table + 32);
    let segment = captured("table.tsv", segments);
    let stride = captured("table.tsv", segment + 48);
    let slots = captured("table.tsv", segment + 56);
    let base = captured("table.tsv", segment + 72);
    let bitmap = captured("bitmap.tsv", captured_bitmap_address());
    let ids: Vec<u64> = game_os_port_ids("4.93", 0, STORAGE_SC).skip(1).collect();
    assert_eq!(ids.len(), ports.len());
    for (&id, &port) in ids.iter().zip(&ports) {
        assert!(id < 1 << (shape >> 32), "id {id} is in segment 0");
        assert!(id < slots, "id {id} is below the slot count");
        assert_eq!(captured("table.tsv", base + id * stride), port, "slot {id}");
        assert_eq!(bitmap >> id & 1, 0, "id {id} is in use");
    }
}

/// The address of doubleword 0 of segment 0's free bitmap, the one row of
/// `bitmap.tsv`.
fn captured_bitmap_address() -> u64 {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(STORAGE_DEVICE_PORT_CAPTURE)
        .join("peek/cech20-cex-493/bitmap.tsv");
    let file = path.display().to_string();
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{file}: {e}"));
    let rows: Vec<&str> = text.lines().skip(1).collect();
    assert_eq!(rows.len(), 1, "{file}: rows");
    let cell = rows[0].split('\t').next().unwrap_or_default();
    let digits = cell
        .strip_prefix("0x")
        .unwrap_or_else(|| panic!("{file}: {cell} is not 0x hex"));
    u64::from_str_radix(digits, 16).unwrap_or_else(|e| panic!("{file}: {cell}: {e}"))
}
