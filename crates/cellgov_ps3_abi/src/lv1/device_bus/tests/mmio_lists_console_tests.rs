//! The mappings of the first and third MMIO regions of device 1 of bus 1,
//! held to the console capture of their lists in lv1.

use std::collections::BTreeMap;

use super::*;
use crate::lv1::memory::region_address;
use crate::lv1::partition::LPAR_ID_GAME_OS;

/// The address of device 1 of bus 1 on the reference console, from the
/// capture `tests/micro/lv1_device_bus_open`.
const DEVICE_1: u64 = 0x61_E050;

/// The class of a mapping, from the TOC slot r2 - 28320
/// [OFW-4.93 lv1 a:0x353198-0x35319C].
const MAPPING_CLASS: u64 = 0x35_C538;

/// The doublewords of the capture file `file` under `capture`, by address.
///
/// The column `address` holds each address and the last column the
/// doubleword. Both reads of each doubleword must agree.
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

/// The doublewords of both files of the lists capture.
fn lists() -> BTreeMap<u64, u64> {
    let mut lists = words(MMIO_LISTS_CAPTURE, "region_lists.tsv");
    let next = words(MMIO_LISTS_CAPTURE, "region_3_next.tsv");
    assert_eq!((lists.len(), next.len()), (27, 12));
    lists.extend(next);
    assert_eq!(lists.len(), 39);
    lists
}

/// The region object in the device slot `slot`, from the capture of the
/// device's regions.
fn region_object(slot: u64) -> u64 {
    words(MMIO_MAP_CAPTURE, "mmio_map.tsv")[&(DEVICE_1 + 24 + 8 * slot)]
}

#[test]
fn the_third_regions_game_os_mapping_is_the_record_of_the_kernels_second_call() {
    let lists = lists();
    let row = GAME_OS_MMIO_MAPPINGS[1];
    let region =
        mmio_region(row.firmware, row.bus, row.device, row.bus_address).expect("the call's region");
    assert_eq!(region.slot, 2);
    let object = region_object(region.slot);
    let head = lists[&(object + 72)];
    assert_eq!(
        head,
        words(MMIO_MAP_CAPTURE, "mmio_map.tsv")[&(object + 72)]
    );

    assert_eq!(lists[&head], MAPPING_CLASS);
    assert_eq!(lists[&(head + 8)], LPAR_ID_GAME_OS);
    assert_eq!(lists[&(head + 24)], 0);
    assert_eq!(lists[&(head + 48)], row.lpar_address);
    assert_eq!(lists[&(head + 56)], row.size);
    assert_eq!(lists[&(head + 64)] >> 56, 1);
    assert_eq!(lists[&(head + 72)], object);
    assert_eq!(lists[&(head + 80)], row.lpar_address);

    let next = lists[&(head + 32)];
    assert_eq!(lists[&next], MAPPING_CLASS);
    assert_eq!(lists[&(next + 8)], 0);
    assert_eq!(lists[&(next + 24)], head);
    assert_eq!(lists[&(next + 32)], 0);
    assert_eq!(
        Some(lists[&(next + 48)]),
        mmio_real_address(row.firmware, row.bus, row.device, row.bus_address)
    );
    assert_eq!(lists[&(next + 64)] >> 56, 0);
    assert_eq!(lists[&(next + 72)], object);
}

#[test]
fn the_first_region_holds_only_lv1s_own_mapping() {
    let lists = lists();
    let region = mmio_region("4.93", 1, 1, 0x2400).expect("the first region");
    let object = region_object(region.slot);
    let head = lists[&(object + 72)];
    assert_eq!(lists[&head], MAPPING_CLASS);
    assert_eq!(lists[&(head + 8)], 0);
    assert_eq!(lists[&(head + 32)], 0);
    assert_eq!(
        Some(lists[&(head + 48)]),
        mmio_real_address(region.firmware, region.bus, region.device, region.address)
    );
    assert_eq!(lists[&(head + 72)], object);
    assert!(GAME_OS_MMIO_MAPPINGS
        .iter()
        .all(|row| row.bus_address != region.address));
}

#[test]
fn the_kernels_device_1_mappings_take_the_region_counter_values_7_and_8() {
    let rows: Vec<_> = GAME_OS_MMIO_MAPPINGS
        .iter()
        .filter(|row| row.firmware == "4.93" && row.device == 1)
        .collect();
    assert_eq!(rows.len(), 2);
    for (counter, row) in (7u64..).zip(rows) {
        assert_eq!(
            region_address(row.size, counter),
            Some(row.lpar_address),
            "{:#x}",
            row.bus_address
        );
    }
}
