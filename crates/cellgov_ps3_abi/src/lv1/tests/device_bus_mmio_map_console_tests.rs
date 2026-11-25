//! The 4.93 MMIO regions of device 1 of bus 1 and the mapping of the
//! kernel's call, held to the console capture of the device in lv1.

use super::*;
use crate::lv1::partition::LPAR_ID_GAME_OS;

/// The address of device 1 of bus 1 on the reference console, from the
/// capture `tests/micro/lv1_device_bus_open`.
const DEVICE_1: u64 = 0x61_E050;

/// The method table of the bus 1 devices
/// [OFW-4.93 lv1 a:0x353480-0x353484].
const DEVICE_TABLE: u64 = 0x35_C878;

/// The class of a region, from the TOC slot r2 - 28312
/// [OFW-4.93 lv1 a:0x3531A0-0x3531A4].
const REGION_CLASS: u64 = 0x35_C560;

/// The class of a mapping, from the TOC slot r2 - 28320
/// [OFW-4.93 lv1 a:0x353198-0x35319C].
const MAPPING_CLASS: u64 = 0x35_C538;

/// The page shift that the region function gives each region
/// [OFW-4.93 lv1 a:0x227FBC].
const REGION_PAGE_SHIFT: u64 = 20;

/// The rows of `mmio_map.tsv`, as (field, address, value). Both reads of
/// each doubleword must agree.
fn rows() -> Vec<(String, u64, u64)> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(MMIO_MAP_CAPTURE)
        .join("peek/cech20-cex-493/mmio_map.tsv");
    let name = path.display().to_string();
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{name}: {e}"));
    let mut lines = text.lines();
    assert_eq!(
        lines.next(),
        Some("address\tfield\twidth\treads\tagree\tvalue"),
        "{name}: header"
    );
    let hex = |text: &str| {
        u64::from_str_radix(text.trim_start_matches("0x"), 16)
            .unwrap_or_else(|e| panic!("{name}: {text}: {e}"))
    };
    lines
        .map(|line| {
            let row: Vec<&str> = line.split('\t').collect();
            assert_eq!(row.len(), 6, "{name}: {line}");
            assert_eq!(
                (row[2], row[3], row[4]),
                ("8", "2", "yes"),
                "{name}: {line}"
            );
            (row[1].to_string(), hex(row[0]), hex(row[5]))
        })
        .collect()
}

/// The value of the field `field`, after the check that it is at
/// `address`.
fn at(rows: &[(String, u64, u64)], field: &str, address: u64) -> u64 {
    let (row, value) = rows
        .iter()
        .find(|(name, _, _)| name == field)
        .map(|&(_, address, value)| (address, value))
        .unwrap_or_else(|| panic!("no row for {field}"));
    assert_eq!(row, address, "the {field} row");
    value
}

#[test]
fn the_device_slots_hold_the_regions_of_lv1s_init_and_no_other() {
    let rows = rows();
    assert_eq!(rows.len(), 63);
    assert_eq!(at(&rows, "device_table", DEVICE_1), DEVICE_TABLE);
    let slots: Vec<u64> = (0..8u64)
        .map(|slot| {
            at(
                &rows,
                &format!("device_slot_{slot}"),
                DEVICE_1 + 24 + 8 * slot,
            )
        })
        .collect();
    assert_eq!(slots.len(), 8);
    assert!(slots[3..].iter().all(|&slot| slot == 0), "{slots:x?}");

    let regions: Vec<_> = MMIO_REGIONS
        .iter()
        .filter(|r| r.firmware == "4.93" && r.bus == 1 && r.device == 1)
        .collect();
    assert_eq!(regions.len(), 3);
    for region in regions {
        let n = region.slot + 1;
        let object = slots[usize::try_from(region.slot).expect("a slot index")];
        let field =
            |name: &str, offset: u64| at(&rows, &format!("region_{n}_{name}"), object + offset);
        assert_eq!(field("class", 0), REGION_CLASS, "region {n}");
        assert_eq!(field("partition", 8), 0, "region {n}");
        assert_eq!(field("base_16", 16), region.address, "region {n}");
        assert_eq!(field("address", 48), region.address, "region {n}");
        assert_eq!(field("size", 56), region.size, "region {n}");
        assert_eq!(field("page_shift", 64), REGION_PAGE_SHIFT, "region {n}");
    }
}

#[test]
fn the_kernel_call_mapping_is_the_one_game_os_mapping_of_its_region() {
    let rows = rows();
    let row = GAME_OS_MMIO_MAPPINGS[0];
    assert_eq!(GAME_OS_MMIO_MAPPINGS.len(), 2);
    let region =
        mmio_region(row.firmware, row.bus, row.device, row.bus_address).expect("the call's region");
    let object = at(&rows, "device_slot_1", DEVICE_1 + 24 + 8 * region.slot);
    assert_eq!(at(&rows, "region_2_address", object + 48), row.bus_address);

    let first = at(&rows, "region_2_72", object + 72);
    assert_eq!(at(&rows, "mapping_1_class", first), MAPPING_CLASS);
    assert_eq!(at(&rows, "mapping_1_partition", first + 8), LPAR_ID_GAME_OS);
    assert_eq!(at(&rows, "mapping_1_offset_16", first + 16), 0);
    assert_eq!(at(&rows, "mapping_1_previous", first + 24), 0);
    assert_eq!(at(&rows, "mapping_1_offset", first + 40), 0);
    let lpar = at(&rows, "mapping_1_lpar", first + 48);
    assert_eq!(at(&rows, "mapping_1_length", first + 56), row.size);
    assert_eq!(at(&rows, "mapping_1_64", first + 64) >> 56, 1);
    assert_eq!(at(&rows, "mapping_1_region", first + 72), object);
    assert_eq!(at(&rows, "mapping_1_lpar_80", first + 80), lpar);
    assert_eq!(row.lpar_address, lpar);

    let second = at(&rows, "mapping_1_next", first + 32);
    assert_eq!(at(&rows, "mapping_2_class", second), MAPPING_CLASS);
    assert_eq!(at(&rows, "mapping_2_partition", second + 8), 0);
    assert_eq!(at(&rows, "mapping_2_previous", second + 24), first);
    assert_eq!(at(&rows, "mapping_2_next", second + 32), 0);
    assert_eq!(at(&rows, "mapping_2_region", second + 72), object);
    assert_eq!(at(&rows, "mapping_2_64", second + 64) >> 56, 0);
}
