//! The 4.93 mappings of the kernel's DMA map calls for devices 1 and 2 of
//! bus 1, held to the console capture of the devices' mapping lists in
//! lv1.

use std::collections::{BTreeMap, BTreeSet};

use super::*;
use crate::lv1::partition::LPAR_ID_GAME_OS;

/// The global that holds the system bus object, from the TOC slot
/// r2 - 27872 [OFW-4.93 lv1 a:0x353358-0x35335C].
const SYSTEM_BUS_GLOBAL: u64 = 0x36_82C0;

/// The slots of the system bus object that the lookup of bus 1 takes for
/// devices 1 and 2: +32 + 8 x 3 and +32 + 8 x 4
/// [OFW-4.93 lv1 a:0x235DA4-0x235DB8] [OFW-4.93 lv1 a:0x22B838-0x22B844].
const DEVICE_SLOTS: [(u64, u64); 2] = [(1, 56), (2, 64)];

/// The offset of the head of a device's region list
/// [OFW-4.93 lv1 a:0x226C8C-0x226D0C].
const REGION_LIST: u64 = 96;

/// The offset of the head of a region's mapping list
/// [OFW-4.93 lv1 a:0x225EB8] [OFW-4.93 lv1 a:0x225F50].
const MAPPING_LIST: u64 = 72;

/// The most regions that the slot object of device 1 or 2 holds, which
/// bounds each walk of a region list.
const REGION_BOUND: usize = DMA_SLOT_COUNT;

/// A bound on each walk of a mapping list: the capture's longest list
/// holds 7 mappings.
const MAPPING_BOUND: usize = 16;

/// The words of the capture file `file`, by address.
///
/// The column `address` holds each address and the last column the
/// word. Both reads of each word must agree.
fn words(file: &str) -> BTreeMap<u64, u64> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(DMA_MAPPINGS_CAPTURE)
        .join("peek/cech20-cex-493")
        .join(file);
    let name = path.display().to_string();
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{name}: {e}"));
    let mut lines = text.lines();
    let header: Vec<&str> = lines.next().expect("a header").split('\t').collect();
    assert_eq!(header[..3], ["address", "reads", "agree"], "{name}: header");
    let hex = |text: &str| {
        u64::from_str_radix(text.trim_start_matches("0x"), 16)
            .unwrap_or_else(|e| panic!("{name}: {text}: {e}"))
    };
    let mut words = BTreeMap::new();
    for line in lines {
        let row: Vec<&str> = line.split('\t').collect();
        assert_eq!(row.len(), header.len(), "{name}: {line}");
        assert_eq!((row[1], row[2]), ("2", "yes"), "{name}: {line}");
        assert_eq!(
            words.insert(hex(row[0]), hex(row[3])),
            None,
            "{name}: {line}"
        );
    }
    words
}

/// The doublewords of the capture: the globals, the devices, the regions
/// and the mappings.
fn doublewords() -> BTreeMap<u64, u64> {
    let mut all = BTreeMap::new();
    for file in ["globals.tsv", "devices.tsv", "regions.tsv", "mappings.tsv"] {
        all.extend(words(file));
    }
    all
}

/// The device of each device id, from the system bus object.
fn devices(words: &BTreeMap<u64, u64>) -> Vec<(u64, u64)> {
    let system_bus = words[&SYSTEM_BUS_GLOBAL];
    DEVICE_SLOTS
        .iter()
        .map(|&(device, slot)| (device, words[&(system_bus + slot)]))
        .collect()
}

/// The list that starts at the doubleword `head`, through the next link
/// at +32 of each element, after the check that it ends within `bound`
/// elements.
fn list(words: &BTreeMap<u64, u64>, head: u64, bound: usize) -> Vec<u64> {
    let mut elements = Vec::new();
    let mut element = words[&head];
    for _ in 0..bound {
        if element == 0 {
            return elements;
        }
        elements.push(element);
        element = words[&(element + 32)];
    }
    assert_eq!(element, 0, "the list at 0x{head:X} ends within {bound}");
    elements
}

/// The mappings in each region of `device`, after the check that each
/// region and mapping is the game OS partition's.
fn captured_mappings(
    words: &BTreeMap<u64, u64>,
    device: u64,
    object: u64,
) -> Vec<GameOsDmaMapping> {
    let mut mappings = Vec::new();
    for region in list(words, object + REGION_LIST, REGION_BOUND) {
        assert_eq!(words[&(region + 8)], LPAR_ID_GAME_OS, "region 0x{region:X}");
        let address = words[&(region + 48)];
        for mapping in list(words, region + MAPPING_LIST, MAPPING_BOUND) {
            assert_eq!(
                words[&(mapping + 8)],
                LPAR_ID_GAME_OS,
                "mapping 0x{mapping:X}"
            );
            assert_eq!(words[&(mapping + 72)], region, "mapping 0x{mapping:X}");
            mappings.push(GameOsDmaMapping {
                firmware: "4.93",
                bus: 1,
                device,
                bus_address: address + words[&(mapping + 40)],
                lpar_address: words[&(mapping + 48)],
                length: words[&(mapping + 56)],
            });
        }
    }
    mappings
}

/// The key of a mapping for a set comparison.
fn key(row: &GameOsDmaMapping) -> (u64, u64, u64, u64) {
    (row.device, row.bus_address, row.lpar_address, row.length)
}

#[test]
fn each_kernel_page_of_devices_1_and_2_is_a_captured_mapping_of_its_device() {
    let words = doublewords();
    let mut captured = BTreeSet::new();
    for (device, object) in devices(&words) {
        for mapping in captured_mappings(&words, device, object) {
            assert!(captured.insert(key(&mapping)), "{mapping:?}");
        }
    }
    let rows: BTreeSet<_> = GAME_OS_DMA_MAPPINGS
        .iter()
        .filter(|row| row.firmware == "4.93" && row.device != 0)
        .map(key)
        .collect();
    assert_eq!(rows.len(), 18);
    assert!(rows.is_subset(&captured), "{rows:X?}");

    let later: BTreeSet<_> = captured.difference(&rows).copied().collect();
    let expected = BTreeSet::from([
        (1, 0xC080_0000, 0x9A_0000, 0x1_0000),
        (2, 0xD000_0000, 0x80_0000, 0x10_0000),
    ]);
    assert_eq!(later, expected);
}

#[test]
fn the_word_at_160_of_devices_1_and_2_lets_the_entry_give_an_io_address() {
    let io_words = words("device_160.tsv");
    let objects = devices(&doublewords());
    assert_eq!(objects.len(), 2);
    for (device, object) in objects {
        let word = io_words[&(object + 160)];
        assert_ne!(word, u64::from(DMA_MAP_NO_IO_WORD), "device {device}");
    }
}

#[test]
fn the_region_lists_of_devices_1_and_2_hold_no_region_of_lv1() {
    let words = doublewords();
    let mut regions = 0;
    for (device, object) in devices(&words) {
        for region in list(&words, object + REGION_LIST, REGION_BOUND) {
            regions += 1;
            assert_eq!(words[&(region + 8)], LPAR_ID_GAME_OS, "region 0x{region:X}");
            let address = words[&(region + 48)];
            assert_eq!(lv1_dma_region("4.93", 1, device, address), None);
        }
    }
    assert_eq!(regions, 10);
}
