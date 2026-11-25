//! The 4.93 bus address of the kernel's DMA region call, held to the
//! console capture of the network device's regions in lv1.

use super::*;

/// The lv1 global that holds the system bus object, from the TOC slot
/// r2 - 27872 [OFW-4.93 lv1 a:0x353358-0x35335C].
const SYSTEM_BUS_GLOBAL: u64 = 0x3682c0;

/// The offset in the system bus object of device 0 of bus 1
/// [OFW-4.93 lv1 a:0x22B838-0x22B844].
const DEVICE_0_OFFSET: u64 = 32;

/// The method table of device 0, from the TOC slot that the device's
/// constructor reads [OFW-4.93 lv1 a:0x353480-0x353484].
const DEVICE_0_TABLE: u64 = 0x35c878;

/// The offset in a device of the head of its region list
/// [OFW-4.93 lv1 a:0x226C90] [OFW-4.93 lv1 a:0x226D04].
const REGION_LIST_OFFSET: u64 = 96;

/// The class of a DMA region, from the TOC slot r2 - 28312
/// [OFW-4.93 lv1 a:0x3531A0-0x3531A4].
const REGION_CLASS: u64 = 0x35c560;

/// The offset in a device of the word that refuses an allocation when it
/// holds -1 [OFW-4.93 lv1 a:0x22D990-0x22D9B0].
const REFUSE_OFFSET: u64 = 120;

/// The offset in a device of its pool index, a 32-bit word
/// [OFW-4.93 lv1 a:0x22D9B4].
const POOL_INDEX_OFFSET: u64 = 128;

/// The offset in a device of its pool backend [OFW-4.93 lv1 a:0x22D9B8].
const POOL_BACKEND_OFFSET: u64 = 136;

/// The rows of the capture, as (field, address, value). Both reads of
/// each doubleword must agree.
fn rows() -> Vec<(String, u64, u64)> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(DMA_REGION_CAPTURE)
        .join("peek/cech20-cex-493/dma_region.tsv");
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

/// The partition, the bus address, the size and the page shift of the
/// region at `region`, region `n` of the list, after the checks of its
/// class and its rows.
fn region(rows: &[(String, u64, u64)], n: u32, region: u64) -> (u64, u64, u64, u64) {
    let field = |suffix: &str| captured(rows, &format!("region_{n}_{suffix}"));
    assert_eq!(field("class"), (region, REGION_CLASS), "region {n} class");
    let (address, partition) = field("partition");
    assert_eq!(address, region + 8, "region {n} partition row");
    let (address, base) = field("base_16");
    assert_eq!(address, region + 16, "region {n} +16 row");
    assert_eq!(field("address"), (region + 48, base), "region {n} +48");
    let (address, size) = field("size");
    assert_eq!(address, region + 56, "region {n} size row");
    let (address, page_shift) = field("page_shift");
    assert_eq!(address, region + 64, "region {n} shift row");
    (partition, base, size, page_shift)
}

/// The pool of the device is slot 0 of the system bus object, and +120
/// does not refuse the allocation. The index is the high word of the
/// doubleword at +128.
#[test]
fn the_captured_device_allocates_from_slot_0_of_the_system_bus_object() {
    let rows = rows();
    let (_, bus) = captured(&rows, "system_bus");
    let (_, device) = captured(&rows, "device");
    let (address, refuse) = captured(&rows, "device_120");
    assert_eq!(address, device + REFUSE_OFFSET, "the +120 row");
    assert_ne!(refuse, u64::MAX, "+120 refuses the allocation");
    let (address, index) = captured(&rows, "device_index");
    assert_eq!(address, device + POOL_INDEX_OFFSET, "the index row");
    assert_eq!(index >> 32, 0, "the pool index");
    assert_eq!(
        captured(&rows, "device_backend"),
        (device + POOL_BACKEND_OFFSET, bus),
        "the pool backend"
    );
}

#[test]
fn the_kernel_call_gets_the_address_of_the_game_os_region_of_the_network_device() {
    let rows = rows();
    let (address, bus) = captured(&rows, "system_bus");
    assert_eq!(address, SYSTEM_BUS_GLOBAL, "the system bus global");
    let (address, device) = captured(&rows, "device");
    assert_eq!(address, bus + DEVICE_0_OFFSET, "the device row");
    assert_eq!(captured(&rows, "device_table"), (device, DEVICE_0_TABLE));
    let (address, game_os) = captured(&rows, "device_partition");
    assert_eq!(address, device + 112, "the partition row");
    let (address, first) = captured(&rows, "device_regions");
    assert_eq!(address, device + REGION_LIST_OFFSET, "the list row");

    let (address, second) = captured(&rows, "region_1_next");
    assert_eq!(address, first + 32, "region 1 next row");
    assert_eq!(captured(&rows, "region_2_previous"), (second + 24, first));
    assert_eq!(captured(&rows, "region_2_next"), (second + 32, 0));
    let regions = [region(&rows, 1, first), region(&rows, 2, second)];
    let game_os_regions: Vec<_> = regions
        .iter()
        .filter(|(partition, ..)| *partition == game_os)
        .collect();
    assert_eq!(game_os_regions.len(), 1, "one region of the game OS");
    let &(_, base, size, page_shift) = game_os_regions[0];

    let row = GAME_OS_DMA_REGIONS[0];
    assert_eq!((row.firmware, row.bus, row.device), ("4.93", 1, 0));
    assert_eq!((row.size, row.page_shift), (size, page_shift));
    assert_eq!(
        game_os_dma_region("4.93", 1, 0, size, page_shift),
        Some(base)
    );
}
