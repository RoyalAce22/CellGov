//! The five DMA regions of device 2 of bus 1, held in their order to the
//! console capture of the device's slot object and region list in lv1.

use std::collections::BTreeMap;

use super::dma_slots_console_tests::{slot_object as device_1_slot_object, slots};
use super::*;
use crate::lv1::partition::LPAR_ID_GAME_OS;

/// The global that holds the system bus object, from the TOC slot
/// r2 - 27872 [OFW-4.93 lv1 a:0x353358-0x35335C].
const SYSTEM_BUS_GLOBAL: u64 = 0x36_82C0;

/// Entry 1 of the table of slot objects at 0x3682F8, device 2's
/// [OFW-4.93 lv1 a:0x23710C-0x237120] [OFW-4.93 lv1 a:0x353C88-0x353C8C].
const DEVICE_2_SLOT_ENTRY: u64 = 0x36_8300;

/// The base of the backend's pool slots
/// [OFW-4.93 lv1 a:0x22D3B0-0x22D3F0] `ld r28, 32(r9)`.
const POOL_SLOTS: u64 = 32;

/// The number of region files of the capture.
const REGION_FILES: usize = 5;

/// The words of the capture file `file`, by address.
///
/// The column `address` holds each address and the last column the
/// word. Both reads of each word must agree.
fn words(file: &str) -> BTreeMap<u64, u64> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(MMIO_DEVICE_2_CAPTURE)
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

/// The words of the device, its pointers and its five DMA regions.
fn device_words() -> BTreeMap<u64, u64> {
    let mut all = BTreeMap::new();
    for file in ["globals.tsv", "device_pointers.tsv", "device.tsv"] {
        all.extend(words(file));
    }
    for index in 1..=REGION_FILES {
        all.extend(words(&format!("dma_region_{index}.tsv")));
    }
    all
}

/// The device at +24 of device 2's slot object
/// [OFW-4.93 lv1 a:0x23D528].
fn device_2(words: &BTreeMap<u64, u64>) -> u64 {
    words[&(words[&DEVICE_2_SLOT_ENTRY] + 24)]
}

/// The regions of device 2's list from its head at +96, in list order,
/// as (partition, bus address, size, page shift).
fn listed_regions(words: &BTreeMap<u64, u64>) -> Vec<(u64, u64, u64, u64)> {
    let mut listed = Vec::new();
    let mut previous = 0;
    let mut record = words[&(device_2(words) + 96)];
    while record != 0 {
        assert!(listed.len() < REGION_FILES, "the list does not end");
        assert_eq!(words[&(record + 24)], previous, "0x{record:X}");
        listed.push((
            words[&(record + 8)],
            words[&(record + 48)],
            words[&(record + 56)],
            words[&(record + 64)],
        ));
        previous = record;
        record = words[&(record + 32)];
    }
    listed
}

fn device_2_rows() -> Vec<GameOsDmaRegion> {
    game_os_dma_regions("4.93", 1, 2).collect()
}

#[test]
fn the_device_2_rows_are_the_devices_listed_regions_in_list_order() {
    let rows = device_2_rows();
    let listed = listed_regions(&device_words());
    assert_eq!(rows.len(), 5);
    assert_eq!(listed.len(), rows.len());
    for (place, (row, region)) in rows.iter().zip(&listed).enumerate() {
        assert_eq!(
            *region,
            (LPAR_ID_GAME_OS, row.address, row.size, row.page_shift),
            "region {place}"
        );
    }
}

/// The slot function takes the first free slot
/// [OFW-4.93 lv1 a:0x23D54C-0x23D578] and stores the partition and the
/// low word of the address [OFW-4.93 lv1 a:0x23D5D0-0x23D5FC].
#[test]
fn each_device_2_row_holds_the_slot_of_its_place_and_the_next_slot_is_free() {
    let rows = device_2_rows();
    let object = device_words()[&DEVICE_2_SLOT_ENTRY];
    let (valid, low, partitions) = (
        words("slot_valid.tsv"),
        words("slot_address.tsv"),
        words("slot_partition.tsv"),
    );
    assert_eq!(rows.len(), 5);
    assert!(rows.len() < DMA_SLOT_COUNT);
    for (slot, row) in (0u64..).zip(&rows) {
        let mark = object + 104 + 16 * slot;
        assert_eq!(valid[&mark], 1, "slot {slot}");
        assert_eq!(low[&(mark + 4)], row.address & 0xFFFF_FFFF, "slot {slot}");
        assert_eq!(partitions[&(mark + 8)], LPAR_ID_GAME_OS, "slot {slot}");
    }
    let taken = (0u64..)
        .take(DMA_SLOT_COUNT)
        .filter(|slot| valid[&(object + 104 + 16 * slot)] == 1)
        .count();
    assert_eq!(taken, rows.len());
    assert_eq!(valid[&(object + 104 + 16 * 5)], 0);
}

/// The pool function takes the pool at +32 + 8 x index of the backend
/// [OFW-4.93 lv1 a:0x22D3B0-0x22D3F0], from the index at +128 and the
/// backend at +136 [OFW-4.93 lv1 a:0x22D9B4-0x22D9BC].
#[test]
fn the_pool_of_devices_1_and_2_is_each_device_itself() {
    let words = device_words();
    let bus_object = words[&SYSTEM_BUS_GLOBAL];
    let device = device_2(&words);
    assert_eq!(words[&(device + 136)], bus_object);
    let index = words[&(device + 128)] >> 32;
    assert_eq!(index, 4);
    assert_eq!(words[&(bus_object + POOL_SLOTS + 8 * index)], device);

    let system_bus = slots("system_bus.tsv");
    assert_eq!(system_bus[&SYSTEM_BUS_GLOBAL], bus_object);
    let device_1 = slots("wrapper.tsv")[&(device_1_slot_object() + 24)];
    let device_1_fields = slots("device.tsv");
    assert_eq!(device_1_fields[&(device_1 + 136)], bus_object);
    let device_1_index = device_1_fields[&(device_1 + 128)] >> 32;
    assert_eq!(device_1_index, 3);
    assert_eq!(
        system_bus[&(bus_object + POOL_SLOTS + 8 * device_1_index)],
        device_1
    );
    assert_ne!(device, device_1);
}
