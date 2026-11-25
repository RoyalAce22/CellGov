//! The 4.93 mapping of the kernel's DMA map call and lv1's own DMA
//! region, held to the console captures of the network device in lv1.

use super::*;

/// The class of a DMA mapping, from the TOC slot r2 - 28320
/// [OFW-4.93 lv1 a:0x353198-0x35319C].
const MAPPING_CLASS: u64 = 0x35c538;

/// The offset in a region of the head of its mapping list
/// [OFW-4.93 lv1 a:0x225EB8] [OFW-4.93 lv1 a:0x225F50].
const MAPPING_LIST_OFFSET: u64 = 72;

/// The rows of the capture file `file` under `capture`, as (field,
/// address, value). Both reads of each doubleword must agree.
fn rows(capture: &str, file: &str) -> Vec<(String, u64, u64)> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(capture)
        .join("peek/cech20-cex-493")
        .join(file);
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

/// The address and the value of the field `field`.
fn captured(rows: &[(String, u64, u64)], field: &str) -> (u64, u64) {
    rows.iter()
        .find(|(name, _, _)| name == field)
        .map(|&(_, address, value)| (address, value))
        .unwrap_or_else(|| panic!("no row for {field}"))
}

/// The value of the field `field`, after the check that it is at
/// `address`.
fn at(rows: &[(String, u64, u64)], field: &str, address: u64) -> u64 {
    let (row, value) = captured(rows, field);
    assert_eq!(row, address, "the {field} row");
    value
}

#[test]
fn the_kernel_call_mapping_is_the_one_mapping_of_the_game_os_region() {
    let regions = rows(DMA_REGION_CAPTURE, "dma_region.tsv");
    let maps = rows(DMA_MAP_CAPTURE, "dma_map.tsv");
    let (_, game_os) = captured(&regions, "device_partition");
    let (_, region) = captured(&regions, "region_1_next");
    assert_eq!(at(&regions, "region_2_partition", region + 8), game_os);
    let region_address = at(&regions, "region_2_address", region + 48);

    let mapping = at(&maps, "region_2_72", region + MAPPING_LIST_OFFSET);
    assert_eq!(at(&maps, "mapping_1_class", mapping), MAPPING_CLASS);
    assert_eq!(at(&maps, "mapping_1_partition", mapping + 8), game_os);
    assert_eq!(at(&maps, "mapping_1_previous", mapping + 24), 0);
    assert_eq!(at(&maps, "mapping_1_next", mapping + 32), 0);
    let offset = at(&maps, "mapping_1_offset", mapping + 40);
    assert_eq!(at(&maps, "mapping_1_offset_16", mapping + 16), offset);
    let lpar = at(&maps, "mapping_1_lpar", mapping + 48);
    let length = at(&maps, "mapping_1_length", mapping + 56);
    assert_eq!(at(&maps, "mapping_1_64", mapping + 64) >> 56, 1);
    assert_eq!(at(&maps, "mapping_1_region", mapping + 72), region);
    assert_eq!(at(&maps, "mapping_1_lpar_80", mapping + 80), lpar);

    assert_eq!(GAME_OS_DMA_MAPPINGS.len(), 1);
    let row = GAME_OS_DMA_MAPPINGS[0];
    assert_eq!((row.firmware, row.bus, row.device), ("4.93", 1, 0));
    assert_eq!(
        (row.bus_address, row.lpar_address, row.length),
        (region_address + offset, lpar, length)
    );
}

#[test]
fn lv1s_own_region_is_region_1_of_the_capture_with_partition_0() {
    let regions = rows(DMA_REGION_CAPTURE, "dma_region.tsv");
    let (_, first) = captured(&regions, "device_regions");
    assert_eq!(at(&regions, "region_1_partition", first + 8), 0);
    let address = at(&regions, "region_1_address", first + 48);
    let size = at(&regions, "region_1_size", first + 56);
    let page_shift = at(&regions, "region_1_page_shift", first + 64);
    assert_eq!(LV1_DMA_REGIONS.len(), 1);
    let row = LV1_DMA_REGIONS[0];
    assert_eq!((row.firmware, row.bus, row.device), ("4.93", 1, 0));
    assert_eq!(
        (row.address, row.size, row.page_shift),
        (address, size, page_shift)
    );
}
