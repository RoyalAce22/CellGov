//! The MMIO regions of device 2 of bus 1 and their mappings, held to the
//! console capture of the device in lv1.

use std::collections::BTreeMap;

use super::*;
use crate::lv1::memory::region_address;
use crate::lv1::partition::LPAR_ID_GAME_OS;

/// The global that holds the system bus object, from the TOC slot
/// r2 - 27872 [OFW-4.93 lv1 a:0x353358-0x35335C].
const SYSTEM_BUS_GLOBAL: u64 = 0x36_82C0;

/// The offset of device 2 in the system bus object
/// [OFW-4.93 lv1 a:0x3A9AC4] `std r3, 64(r31)`.
const DEVICE_2_SLOT: u64 = 64;

/// The class of a region, from the TOC slot r2 - 28312
/// [OFW-4.93 lv1 a:0x3531A0-0x3531A4].
const REGION_CLASS: u64 = 0x35_C560;

/// The class of a mapping, from the TOC slot r2 - 28320
/// [OFW-4.93 lv1 a:0x353198-0x35319C].
const MAPPING_CLASS: u64 = 0x35_C538;

/// The page shift of each region of lv1's init
/// [OFW-4.93 lv1 a:0x227F4C-0x227FC8].
const REGION_PAGE_SHIFT: u64 = 20;

/// The number of region slots of a device
/// [OFW-4.93 lv1 a:0x228C70-0x228CA0].
const REGION_SLOTS: u64 = 8;

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

/// The words of the MMIO files of the capture: the device, its regions
/// and the mappings of their lists.
fn mmio_words() -> BTreeMap<u64, u64> {
    let files = [
        ("globals.tsv", 3),
        ("device_pointers.tsv", 2),
        ("device.tsv", 14),
        ("mmio_regions.tsv", 30),
        ("mmio_heads.tsv", 36),
        ("mmio_next.tsv", 24),
    ];
    let mut all = BTreeMap::new();
    for (file, rows) in files {
        let words = words(file);
        assert_eq!(words.len(), rows, "{file}");
        all.extend(words);
    }
    assert_eq!(all.len(), 109);
    all
}

/// The address of device 2, from +64 of the system bus object.
fn device_2(words: &BTreeMap<u64, u64>) -> u64 {
    words[&(words[&SYSTEM_BUS_GLOBAL] + DEVICE_2_SLOT)]
}

/// The rows of `GAME_OS_MMIO_MAPPINGS` for device 2, in table order.
fn device_2_mappings() -> Vec<GameOsMmioMapping> {
    let rows: Vec<_> = GAME_OS_MMIO_MAPPINGS
        .iter()
        .filter(|row| (row.firmware, row.bus, row.device) == ("4.93", 1, 2))
        .copied()
        .collect();
    assert_eq!(rows.len(), 2);
    rows
}

#[test]
fn device_2_has_index_4_so_each_region_lies_at_the_real_base_plus_its_bus_address() {
    let words = mmio_words();
    let device = device_2(&words);
    // The constructor stores the index as a word at +128
    // [OFW-4.93 lv1 a:0x22DDD8] `stw r26, 128(r31)`.
    assert_eq!(words[&(device + 128)] >> 32, 4);
    let regions: Vec<_> = MMIO_REGIONS.iter().filter(|r| r.device == 2).collect();
    assert_eq!(regions.len(), 3);
    for region in regions {
        assert_eq!(
            mmio_real_address(region.firmware, region.bus, region.device, region.address),
            Some(MMIO_REAL_BASE + region.address),
            "slot {}",
            region.slot
        );
    }
}

/// [OFW-4.93 lv1 a:0x3A9AB4-0x3A9AF0] `li r4, i` with
/// `li r5, 9728; li r6, 512`, `lis r5, 0x302; lis r6, 1` and
/// `lis r5, 0x382; lis r6, 1`.
#[test]
fn device_2s_slots_hold_the_three_regions_of_lv1s_init_and_no_other() {
    let words = mmio_words();
    let device = device_2(&words);
    let regions: Vec<_> = MMIO_REGIONS
        .iter()
        .filter(|r| (r.firmware, r.bus, r.device) == ("4.93", 1, 2))
        .map(|r| (r.slot, r.address, r.size))
        .collect();
    assert_eq!(
        regions,
        [
            (0, 0x2600, 0x200),
            (1, 0x302_0000, 0x1_0000),
            (2, 0x382_0000, 0x1_0000)
        ]
    );
    for (slot, address, size) in regions {
        let object = words[&(device + 24 + 8 * slot)];
        assert_eq!(words[&object], REGION_CLASS, "slot {slot}");
        assert_eq!(words[&(object + 8)], 0, "slot {slot}");
        assert_eq!(words[&(object + 16)], address, "slot {slot}");
        assert_eq!(words[&(object + 48)], address, "slot {slot}");
        assert_eq!(words[&(object + 56)], size, "slot {slot}");
        assert_eq!(words[&(object + 64)], REGION_PAGE_SHIFT, "slot {slot}");
    }
    for slot in 3..REGION_SLOTS {
        assert_eq!(words[&(device + 24 + 8 * slot)], 0, "slot {slot}");
    }
}

#[test]
fn each_device_2_row_is_the_game_os_mapping_at_the_head_of_its_regions_list() {
    let words = mmio_words();
    let device = device_2(&words);
    for row in device_2_mappings() {
        let region = mmio_region(row.firmware, row.bus, row.device, row.bus_address)
            .expect("the call's region");
        let object = words[&(device + 24 + 8 * region.slot)];
        let head = words[&(object + 72)];
        let at = |offset: u64| words[&(head + offset)];
        assert_eq!(at(0), MAPPING_CLASS, "{:#x}", row.bus_address);
        assert_eq!(at(8), LPAR_ID_GAME_OS, "{:#x}", row.bus_address);
        assert_eq!(at(24), 0, "{:#x}", row.bus_address);
        assert_eq!(at(48), row.lpar_address, "{:#x}", row.bus_address);
        assert_eq!(at(56), row.size, "{:#x}", row.bus_address);
        assert_eq!(at(64) >> 56, 1, "{:#x}", row.bus_address);
        assert_eq!(at(72), object, "{:#x}", row.bus_address);
        assert_eq!(at(80), row.lpar_address, "{:#x}", row.bus_address);

        // The one other mapping is lv1's own, at the region's real address.
        let next = at(32);
        assert_eq!(words[&next], MAPPING_CLASS, "{:#x}", row.bus_address);
        assert_eq!(words[&(next + 8)], 0, "{:#x}", row.bus_address);
        assert_eq!(words[&(next + 24)], head, "{:#x}", row.bus_address);
        assert_eq!(words[&(next + 32)], 0, "{:#x}", row.bus_address);
        assert_eq!(
            Some(words[&(next + 48)]),
            mmio_real_address(row.firmware, row.bus, row.device, row.bus_address),
            "{:#x}",
            row.bus_address
        );
    }

    // The first region's list holds only lv1's own mapping.
    let first = mmio_region("4.93", 1, 2, 0x2600).expect("the first region");
    let head = words[&(words[&(device + 24 + 8 * first.slot)] + 72)];
    assert_eq!(words[&(head + 8)], 0);
    assert_eq!(words[&(head + 32)], 0);
    assert_eq!(words[&(head + 48)], MMIO_REAL_BASE + first.address);
    assert!(device_2_mappings()
        .iter()
        .all(|row| row.bus_address != first.address));
}

#[test]
fn device_2s_mappings_take_the_region_counter_values_9_and_10() {
    for (counter, row) in (9u64..).zip(device_2_mappings()) {
        assert_eq!(
            region_address(row.size, counter),
            Some(row.lpar_address),
            "{:#x}",
            row.bus_address
        );
    }
}
