//! The DMA slot object of device 1 of bus 1 and its first region, held
//! to the console capture of the slot object and region list in lv1.

use std::collections::BTreeMap;

use super::*;
use crate::lv1::partition::LPAR_ID_GAME_OS;

/// The table of slot objects, from the TOC slot r2 - 25520
/// [OFW-4.93 lv1 a:0x353C88-0x353C8C].
const SLOT_TABLE: u64 = 0x36_82F8;

/// The global that holds the system bus object, from the TOC slot
/// r2 - 27872 [OFW-4.93 lv1 a:0x353358-0x35335C].
const SYSTEM_BUS_GLOBAL: u64 = 0x36_82C0;

/// The number of slots in a slot object [OFW-4.93 lv1 a:0x23D54C-0x23D578].
const SLOTS: u64 = 6;

/// The most regions that the walk of the device's list follows.
const MAX_REGIONS: usize = 8;

/// The words of the capture file `file` under `capture`, by address.
///
/// The column `address` holds each address and the last column the
/// word. Both reads of each word must agree.
fn words(capture: &str, file: &str) -> BTreeMap<u64, u64> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(capture)
        .join("peek/cech20-cex-493")
        .join(file);
    let name = path.display().to_string();
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{name}: {e}"));
    let mut lines = text.lines();
    let header: Vec<&str> = lines.next().expect("a header").split('\t').collect();
    let column = |label: &str| {
        header
            .iter()
            .position(|cell| *cell == label)
            .unwrap_or_else(|| panic!("{name}: no {label} column"))
    };
    let (reads, agree) = (column("reads"), column("agree"));
    let hex = |text: &str| {
        u64::from_str_radix(text.trim_start_matches("0x"), 16)
            .unwrap_or_else(|e| panic!("{name}: {text}: {e}"))
    };
    let mut words = BTreeMap::new();
    for line in lines {
        let row: Vec<&str> = line.split('\t').collect();
        assert_eq!(row.len(), header.len(), "{name}: {line}");
        assert_eq!((row[reads], row[agree]), ("2", "yes"), "{name}: {line}");
        let value = hex(row[row.len() - 1]);
        assert_eq!(words.insert(hex(row[0]), value), None, "{name}: {line}");
    }
    words
}

pub(super) fn slots(file: &str) -> BTreeMap<u64, u64> {
    words(DMA_SLOTS_CAPTURE, file)
}

/// The slot object of device 1: entry 0 of the table
/// [OFW-4.93 lv1 a:0x237108-0x237120].
pub(super) fn slot_object() -> u64 {
    slots("device_table.tsv")[&SLOT_TABLE]
}

/// The device at +24 of device 1's slot object
/// [OFW-4.93 lv1 a:0x23D528].
fn device() -> u64 {
    slots("wrapper.tsv")[&(slot_object() + 24)]
}

/// The regions of the device's list from its head at +96, in list
/// order, as (record address, partition, bus address, size, page shift).
pub(super) fn listed_regions() -> Vec<(u64, u64, u64, u64, u64)> {
    let regions = slots("regions.tsv");
    let mut listed = Vec::new();
    let mut previous = 0;
    let mut record = slots("device.tsv")[&(device() + 96)];
    while record != 0 {
        assert!(listed.len() < MAX_REGIONS, "the list does not end");
        assert_eq!(regions[&(record + 24)], previous, "0x{record:X}");
        listed.push((
            record,
            regions[&(record + 8)],
            regions[&(record + 48)],
            regions[&(record + 56)],
            regions[&(record + 64)],
        ));
        previous = record;
        record = regions[&(record + 32)];
    }
    listed
}

fn device_1_row() -> GameOsDmaRegion {
    let first = game_os_dma_regions("4.93", 1, 1).next();
    first.expect("a device 1 row")
}

/// The lookup gives device 1 the code 3 [OFW-4.93 lv1 a:0x235DA4-0x235DAC],
/// which selects +56 of the system bus object
/// [OFW-4.93 lv1 a:0x22B838-0x22B844].
#[test]
fn the_slot_objects_device_is_the_open_device_that_the_access_check_finds() {
    let system_bus = slots("system_bus.tsv");
    let bus_object = system_bus[&SYSTEM_BUS_GLOBAL];
    assert_eq!(system_bus[&(bus_object + 56)], device());

    let network = words(DMA_REGION_CAPTURE, "dma_region.tsv");
    assert_eq!(network[&SYSTEM_BUS_GLOBAL], bus_object);
    let network_device = network[&(bus_object + 32)];
    let fields = slots("device.tsv");
    assert_eq!(fields[&device()], network[&network_device]);
    assert_eq!(slots("device_open.tsv")[&(device() + 108)], 1);
    assert_eq!(fields[&(device() + 112)], LPAR_ID_GAME_OS);
    assert_ne!(fields[&(device() + 120)], u64::MAX);
    // The pool index is the word at +128, and the backend is the
    // doubleword at +136 [OFW-4.93 lv1 a:0x22D9B4-0x22D9BC].
    assert_eq!(fields[&(device() + 128)] >> 32, 3);
    assert_eq!(network[&(network_device + 128)] >> 32, 0);
    assert_eq!(fields[&(device() + 136)], bus_object);
}

#[test]
fn the_device_1_row_heads_the_devices_list_and_holds_its_first_slot() {
    let row = device_1_row();
    let listed = listed_regions();
    assert_eq!(listed.len(), 5);
    let (_, partition, address, size, page_shift) = listed[0];
    assert_eq!(
        (partition, address, size, page_shift),
        (LPAR_ID_GAME_OS, row.address, row.size, row.page_shift)
    );
    let shaped = listed
        .iter()
        .filter(|r| (r.1, r.3, r.4) == (LPAR_ID_GAME_OS, row.size, row.page_shift))
        .count();
    assert_eq!(shaped, 1);

    let object = slot_object();
    assert_eq!(slots("slot_valid.tsv")[&(object + 104)], 1);
    assert_eq!(slots("slot_address.tsv")[&(object + 108)], row.address);
    assert_eq!(slots("wrapper.tsv")[&(object + 112)], LPAR_ID_GAME_OS);
    assert_eq!(row.address, 0xC000_0000);
}

/// The slot function takes the first free slot
/// [OFW-4.93 lv1 a:0x23D54C-0x23D578] and stores the partition and the
/// low word of the address [OFW-4.93 lv1 a:0x23D5D0-0x23D5FC].
#[test]
fn each_taken_slot_holds_the_region_of_its_place_in_the_list() {
    let listed = listed_regions();
    let object = slot_object();
    let (valid, low, partitions) = (
        slots("slot_valid.tsv"),
        slots("slot_address.tsv"),
        slots("wrapper.tsv"),
    );
    let taken: Vec<u64> = (0..SLOTS)
        .filter(|i| valid[&(object + 104 + 16 * i)] == 1)
        .collect();
    assert_eq!(taken, [0, 1, 2, 3, 4]);
    assert_eq!(valid[&(object + 104 + 16 * 5)], 0);
    assert_eq!(taken.len(), listed.len());
    for (i, region) in taken.iter().zip(&listed) {
        assert_eq!(
            low[&(object + 108 + 16 * i)],
            region.2 & 0xFFFF_FFFF,
            "slot {i}"
        );
        assert_eq!(partitions[&(object + 112 + 16 * i)], region.1, "slot {i}");
    }
}
