use super::*;
use crate::lv1::call::{self, CallSource, MAP_DEVICE_DMA_REGION};
use crate::lv1::status::{zero_extended, LV1_BUSY, LV1_ILLEGAL_PARAMETER_VALUE, LV1_NO_ENTRY};
use crate::lv1::table::{table_entry, TableEntry};

/// The r3 of a check whose high word holds `mark`.
fn marked(mark: u64, status: i64) -> u64 {
    (mark << 32) | zero_extended(status)
}

/// The row is the `LV1_CALL` row of Linux `lv1call.h` for 176.
#[test]
fn map_device_dma_region_carries_its_header_number_name_and_counts() {
    assert_eq!(MAP_DEVICE_DMA_REGION, 176);
    let row = call::lookup(MAP_DEVICE_DMA_REGION).map(|c| (c.name, c.inputs, c.outputs, c.source));
    assert_eq!(
        row,
        Some(("map_device_dma_region", 6, 0, CallSource::Linux))
    );
}

#[test]
fn the_4_93_handler_is_the_live_table_entry_of_176() {
    assert_eq!(DMA_MAP_HANDLERS.len(), 1);
    let handler = dma_map_handler("4.93").expect("the 4.93 handler");
    assert_eq!(handler.handler, 0x306770);
    assert_eq!(
        table_entry("4.93", MAP_DEVICE_DMA_REGION),
        Some(TableEntry::Handler(handler.handler))
    );
    for firmware in ["3.70", "4.92", ""] {
        assert_eq!(dma_map_handler(firmware), None, "{firmware:?}");
    }
}

/// [OFW-4.93 lv1 a:0x306904-0x306910] `li 1; sldi 32; oris 0xffff; ori 0xffef`;
/// [OFW-4.93 lv1 a:0x306830-0x30683C] `li 32; sldi 32; oris 0xffff; ori 0xffef`;
/// [OFW-4.93 lv1 a:0x236420-0x236428] mark 1 and -6;
/// [OFW-4.93 lv1 a:0x236538] -11 with mark 0;
/// [OFW-4.93 lv1 a:0x227444-0x227448] mark 4 and -6;
/// [OFW-4.93 lv1 a:0x2273E8-0x2273EC] mark 5 and -17;
/// [OFW-4.93 lv1 a:0x2273F8-0x2273FC] mark 4 and -9;
/// [OFW-4.93 lv1 a:0x227408-0x22740C] mark 4 and -17;
/// [OFW-4.93 lv1 a:0x227418-0x22741C] mark 3 and -17.
/// [OFW-4.93 lv1 a:0x236DC8-0x236DE4] sets bit (mark + 31).
#[test]
fn each_dma_map_check_r3_puts_its_mark_over_its_status() {
    let cases = [
        (DMA_MAP_BUS_RANGE_R3, marked(1, LV1_ILLEGAL_PARAMETER_VALUE)),
        (DMA_MAP_FLAGS_R3, marked(32, LV1_ILLEGAL_PARAMETER_VALUE)),
        (DMA_MAP_NO_BUS_R3, marked(1, LV1_NO_ENTRY)),
        (DMA_MAP_NOT_OPEN_R3, 0x0000_0000_FFFF_FFF5),
        (DMA_MAP_NO_REGION_R3, marked(8, LV1_NO_ENTRY)),
        (DMA_MAP_LENGTH_R3, marked(16, LV1_ILLEGAL_PARAMETER_VALUE)),
        (DMA_MAP_BUSY_R3, marked(8, LV1_BUSY)),
        (
            DMA_MAP_BUS_ALIGNMENT_R3,
            marked(8, LV1_ILLEGAL_PARAMETER_VALUE),
        ),
        (
            DMA_MAP_LPAR_ALIGNMENT_R3,
            marked(4, LV1_ILLEGAL_PARAMETER_VALUE),
        ),
    ];
    assert_eq!(cases.len(), 9);
    for (r3, expected) in cases {
        assert_eq!(r3, expected, "0x{expected:016X}");
    }
}

/// [OFW-4.93 lv1 a:0x306820-0x306824] `rldicl r0, r28, 52, 1;
/// rldicl r0, r0, 12, 5` clears bits 0 to 4 and bit 52 of the flags.
#[test]
fn the_flags_mask_keeps_bits_0_to_4_and_bit_52() {
    let flags = 0xFFFF_FFFF_FFFF_FFFFu64;
    let cleared = flags.rotate_left(52) & (u64::MAX >> 1);
    let cleared = cleared.rotate_left(12) & (u64::MAX >> 5);
    assert_eq!(!cleared, DMA_MAP_FLAGS_MASK);
    assert_eq!(DMA_MAP_FLAGS_MASK.count_ones(), 6);
}

/// [OFW-4.93 lv1 a:0x2257C8-0x2257E4] bounds the range by the region;
/// [OFW-4.93 lv1 a:0x225804-0x22583C] tests each mapping;
/// [OFW-4.93 lv1 a:0x2257F8-0x225800] skips each mapping for a length of 0.
#[test]
fn the_range_check_refuses_a_range_past_the_region_or_over_a_mapping() {
    let (address, size) = (0xC000_0000u64, 0x10_0000u64);
    let mapping = [(0x4_0000u64, 0x4_0000u64)];
    let cases = [
        (0xC000_0000, 0x10_0000, &[][..], true),
        (0xBFFF_F000, 0x1000, &[][..], false),
        (0xC000_0000, 0x10_1000, &[][..], false),
        (0xC00F_F000, 0x2000, &[][..], false),
        (0xC000_0000, 0x4_0000, &mapping[..], true),
        (0xC008_0000, 0x8_0000, &mapping[..], true),
        (0xC000_0000, 0x4_1000, &mapping[..], false),
        (0xC004_0000, 0x1000, &mapping[..], false),
        (0xC007_F000, 0x1000, &mapping[..], false),
        (0xC000_0000, 0x10_0000, &mapping[..], false),
        (0xC000_0000, u64::MAX, &[][..], true),
        (0xC004_0000, 0, &mapping[..], true),
    ];
    assert_eq!(cases.len(), 12);
    for (bus, length, mappings, free) in cases {
        assert_eq!(
            dma_map_range_free(address, size, mappings.iter().copied(), bus, length),
            free,
            "0x{bus:X} 0x{length:X} {mappings:?}"
        );
    }
}

/// [OFW-4.93 lv1 a:0x22732C-0x227358] checks the bus address, the logical
/// partition address and the length in that order.
#[test]
fn the_page_checks_run_in_lv1_order_and_pass_the_kernel_call() {
    let call = GAME_OS_DMA_MAPPINGS[0];
    let region = GAME_OS_DMA_REGIONS[0];
    assert_eq!(
        dma_map_alignment_r3(
            call.bus_address,
            call.lpar_address,
            call.length,
            region.page_shift,
        ),
        None,
        "the kernel call"
    );
    let cases = [
        (
            0xC000_1000,
            0x80_1000,
            0x1000,
            20,
            Some(DMA_MAP_BUS_ALIGNMENT_R3),
        ),
        (
            0xC000_0000,
            0x80_1000,
            0x1000,
            20,
            Some(DMA_MAP_LPAR_ALIGNMENT_R3),
        ),
        (0xC000_0000, 0x80_0000, 0x1000, 20, Some(DMA_MAP_LENGTH_R3)),
        (0xA000_1000, 0x1000, 0x2000, 12, None),
        (
            0xA000_0800,
            0x1000,
            0x1000,
            12,
            Some(DMA_MAP_BUS_ALIGNMENT_R3),
        ),
    ];
    assert_eq!(cases.len(), 5);
    for (bus, lpar, length, page_shift, r3) in cases {
        assert_eq!(
            dma_map_alignment_r3(bus, lpar, length, page_shift),
            r3,
            "0x{bus:X} 0x{lpar:X} 0x{length:X} {page_shift}"
        );
    }
}

/// The entry at +120 refuses a bus address outside the window
/// [OFW-4.93 lv1 a:0x22D72C-0x22D760], so no region that the model
/// knows reaches that refusal.
#[test]
fn every_known_region_lies_in_the_io_bus_window() {
    let regions = GAME_OS_DMA_REGIONS
        .iter()
        .map(|r| (r.address, r.size))
        .chain(LV1_DMA_REGIONS.iter().map(|r| (r.address, r.size)));
    let mut count = 0;
    for (address, size) in regions {
        count += 1;
        assert!(DMA_MAP_IO_BUS_ADDRESSES.contains(&address), "0x{address:X}");
        assert!(
            DMA_MAP_IO_BUS_ADDRESSES.contains(&(address + size - 1)),
            "0x{address:X} + 0x{size:X}"
        );
    }
    assert_eq!(count, 2);
}

#[test]
fn lv1_dma_region_gives_the_region_that_holds_the_address() {
    let region = LV1_DMA_REGIONS[0];
    assert_eq!(lv1_dma_region("4.93", 1, 0, 0xA000_0000), Some(&region));
    assert_eq!(lv1_dma_region("4.93", 1, 0, 0xA000_7FFF), Some(&region));
    let others = [
        ("4.93", 1, 0, 0xA000_8000),
        ("4.93", 1, 0, 0x9FFF_FFFF),
        ("4.93", 1, 1, 0xA000_0000),
        ("4.93", 2, 0, 0xA000_0000),
        ("4.92", 1, 0, 0xA000_0000),
    ];
    assert_eq!(others.len(), 5);
    for (firmware, bus, device, address) in others {
        assert_eq!(
            lv1_dma_region(firmware, bus, device, address),
            None,
            "{firmware} {bus} {device} 0x{address:X}"
        );
    }
}
