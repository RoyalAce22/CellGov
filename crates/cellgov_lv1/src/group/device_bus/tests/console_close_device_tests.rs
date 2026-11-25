//! The hypervisor's close of device 0 of bus 1 removes the kernel's DMA
//! mapping and region, as lv1's close entry does for the mapping and the
//! region that the committed captures `tests/micro/lv1_device_dma_region`
//! and `tests/micro/lv1_device_dma_map` read on the reference console.

use super::*;
use crate::Hypervisor;
use cellgov_event::UnitId;
use cellgov_ps3_abi::lv1::call::{ALLOCATE_DEVICE_DMA_REGION, MAP_DEVICE_DMA_REGION, OPEN_DEVICE};
use cellgov_ps3_abi::lv1::device_bus::{
    DMA_MAP_CAPTURE, DMA_REGION_CAPTURE, GAME_OS_DMA_MAPPINGS, GAME_OS_DMA_REGIONS,
};
use cellgov_time::GuestTicks;

/// The offset in a region of its list of mappings
/// [OFW-4.93 lv1 a:0x2276B8-0x2276C4].
const MAPPING_LIST: u64 = 72;

/// The offsets in a mapping of its partition, its next link and the word
/// that the unmap step tests [OFW-4.93 lv1 a:0x226AE0-0x226AF8].
const MAPPING_PARTITION: u64 = 8;
const MAPPING_NEXT: u64 = 32;
const MAPPING_88: u64 = 88;

/// The flags that the kernel's map wrapper builds from a flags word of 0
/// [OFW-4.93 lv2_kernel a:0x800000000007d50c-0x800000000007d5bc].
const KERNEL_FLAGS: u64 = 0xE000_0000_0000_0000;

/// The (field, address, value) rows of `file` in the peek capture
/// `capture`. Each row must hold two reads that agree.
fn rows(capture: &str, file: &str) -> Vec<(String, u64, u64)> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(capture)
        .join("peek/cech20-cex-493")
        .join(file);
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let mut lines = text.lines();
    assert_eq!(
        lines.next(),
        Some("address\tfield\twidth\treads\tagree\tvalue"),
        "{}",
        path.display()
    );
    lines
        .map(|line| {
            let cells: Vec<&str> = line.split('\t').collect();
            assert_eq!(cells[3..5], ["2", "yes"], "{file}: {line}");
            (cells[1].to_owned(), hex(cells[0]), hex(cells[5]))
        })
        .collect()
}

fn hex(cell: &str) -> u64 {
    let digits = cell
        .strip_prefix("0x")
        .unwrap_or_else(|| panic!("{cell} is not 0x hex"));
    u64::from_str_radix(digits, 16).unwrap_or_else(|e| panic!("{cell}: {e}"))
}

/// The value of the row `field`, after a check that it is at `address`.
fn at(rows: &[(String, u64, u64)], field: &str, address: u64) -> u64 {
    let (_, row_at, value) = rows
        .iter()
        .find(|(name, _, _)| name == field)
        .unwrap_or_else(|| panic!("no {field} row"));
    assert_eq!(*row_at, address, "the {field} row");
    *value
}

fn value(rows: &[(String, u64, u64)], field: &str) -> (u64, u64) {
    let (_, address, value) = rows
        .iter()
        .find(|(name, _, _)| name == field)
        .unwrap_or_else(|| panic!("no {field} row"));
    (*address, *value)
}

fn kernel_request(number: u64, registers: &[u64]) -> HypercallRequest {
    let mut inputs = [0; 8];
    inputs[..registers.len()].copy_from_slice(registers);
    HypercallRequest {
        unit: UnitId::new(0),
        number,
        inputs,
        pc: 0x8000_0000_000A_357C,
        lr: 0,
        thread: Some(0),
        problem_state: false,
        tick: GuestTicks::ZERO,
    }
}

fn r3_of(disposition: &HypercallDisposition) -> u64 {
    match disposition {
        HypercallDisposition::Answered(answer) => answer.r3,
        HypercallDisposition::Refused(refusal) => panic!("{refusal:?}"),
    }
}

/// The captured mapping is the one mapping of the game OS partition's
/// region, its partition is the caller's and its word at +88 is 0. So
/// lv1's unmap step removes it, and the free step then frees the region.
#[test]
fn the_close_removes_the_captured_mapping_and_region_as_lv1_does() {
    let regions = rows(DMA_REGION_CAPTURE, "dma_region.tsv");
    let maps = rows(DMA_MAP_CAPTURE, "dma_map.tsv");
    let (_, game_os) = value(&regions, "device_partition");
    let (_, region) = value(&regions, "region_1_next");
    assert_eq!(at(&regions, "region_2_partition", region + 8), game_os);
    let mapping = at(&maps, "region_2_72", region + MAPPING_LIST);
    assert_eq!(
        at(&maps, "mapping_1_partition", mapping + MAPPING_PARTITION),
        game_os
    );
    assert_eq!(at(&maps, "mapping_1_next", mapping + MAPPING_NEXT), 0);
    assert_eq!(at(&maps, "mapping_1_88", mapping + MAPPING_88), 0);

    let (region_row, mapping_row) = (GAME_OS_DMA_REGIONS[0], GAME_OS_DMA_MAPPINGS[0]);
    let mut hv = Hypervisor::new();
    hv.set_firmware("4.93");
    let steps: [(u64, &[u64]); 3] = [
        (OPEN_DEVICE, &[1, 0, 0]),
        (
            ALLOCATE_DEVICE_DMA_REGION,
            &[1, 0, region_row.size, region_row.page_shift, 0],
        ),
        (
            MAP_DEVICE_DMA_REGION,
            &[
                1,
                0,
                mapping_row.lpar_address,
                mapping_row.bus_address,
                mapping_row.length,
                KERNEL_FLAGS,
            ],
        ),
    ];
    for (number, registers) in steps {
        assert_eq!(
            r3_of(&hv.dispatch(&kernel_request(number, registers))),
            0,
            "hypercall {number}"
        );
    }
    let device_bus = hv.groups().device_bus();
    assert_eq!(device_bus.dma_regions().count(), 1);
    assert_eq!(device_bus.dma_mappings().count(), 1);

    let close = hv.dispatch(&kernel_request(CLOSE_DEVICE, &[1, 0]));
    assert_eq!(r3_of(&close), 0, "close_device");
    let device_bus = hv.groups().device_bus();
    assert_eq!(device_bus.dma_mappings().count(), 0, "the mappings");
    assert_eq!(device_bus.dma_regions().count(), 0, "the regions");
    assert!(!device_bus.is_open(NETWORK_DEVICE));
}
