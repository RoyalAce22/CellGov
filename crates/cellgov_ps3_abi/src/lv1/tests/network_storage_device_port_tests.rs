//! The storage devices whose interrupt port path CellGov read, and the
//! method table of each 4.93 device, held to the console captures of the
//! storage device objects.

use super::*;
use crate::lv1::device_bus::STORAGE_DEVICE_OPEN_CAPTURE;
use crate::lv1::event_port::STORAGE_DEVICE_PORT_CAPTURE;

/// The global that holds the storage bus object, from the TOC slot at
/// r2 - 26656 [OFW-4.93 lv1 a:0x353818-0x35381C].
const STORAGE_BUS_GLOBAL: u64 = 0x36_8388;

/// The offset in the storage bus object of the device object of device 0.
/// Device `id` is at this offset plus 8 x `id`
/// [OFW-4.93 lv1 a:0x2364F8-0x236500].
const DEVICES_OFFSET: u64 = 3816;

/// The method table of each of devices 0 to 4, in device order. The entry
/// at +48 of each holds the descriptor 0x338778 of the storage bus
/// object's partition method.
const ACCESS_TABLES: [u64; 5] = [
    0x35_E518, // [OFW-4.93 lv1 a:0x35E548-0x35E54C]
    0x35_E6E8, // [OFW-4.93 lv1 a:0x35E718-0x35E71C]
    0x35_E370, // [OFW-4.93 lv1 a:0x35E3A0-0x35E3A4]
    0x35_E1C8, // [OFW-4.93 lv1 a:0x35E1F8-0x35E1FC]
    0x35_DFF8, // [OFW-4.93 lv1 a:0x35E028-0x35E02C]
];

/// The value of the doubleword at `address` in the file `name` of the
/// peek capture `capture`. Both reads of the doubleword must agree.
fn captured(capture: &str, name: &str, address: u64) -> u64 {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(capture)
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

#[test]
fn the_read_storage_device_ports_are_the_devices_of_the_4_93_bus_table() {
    let table = registration("4.93").expect("the 4.93 bus table");
    let read: Vec<u64> = (0..=STORAGE_DEVICE_ID_MAX)
        .filter(|&device| storage_device_port_read("4.93", device))
        .collect();
    let ids: Vec<u64> = table.devices.iter().map(|device| device.id).collect();
    assert_eq!(read, [0, 1, 2, 3, 4]);
    assert_eq!(read, ids);
    for device in [STORAGE_DEVICE_ID_MAX + 1, u64::MAX, 0x1_0000_0000] {
        assert!(!storage_device_port_read("4.93", device), "{device:#x}");
    }
    assert!(!storage_device_port_read("4.92", 0));
}

#[test]
fn each_read_4_93_storage_device_has_a_method_table_with_the_storage_bus_access_method() {
    let bus = captured(STORAGE_DEVICE_PORT_CAPTURE, "bus.tsv", STORAGE_BUS_GLOBAL);
    let devices: Vec<u64> = (0..=STORAGE_DEVICE_ID_MAX)
        .filter(|&device| storage_device_port_read("4.93", device))
        .collect();
    assert_eq!(devices.len(), ACCESS_TABLES.len());
    let tables: Vec<u64> = devices
        .iter()
        .map(|&device| {
            let object = captured(
                STORAGE_DEVICE_PORT_CAPTURE,
                "devices.tsv",
                bus + DEVICES_OFFSET + 8 * device,
            );
            assert_ne!(object, 0, "device {device}: a null device object");
            captured(STORAGE_DEVICE_OPEN_CAPTURE, "walk.tsv", object)
        })
        .collect();
    assert_eq!(tables, ACCESS_TABLES);
}
