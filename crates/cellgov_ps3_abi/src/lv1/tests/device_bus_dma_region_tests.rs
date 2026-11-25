use super::*;
use crate::lv1::call::{self, CallSource, ALLOCATE_DEVICE_DMA_REGION};
use crate::lv1::status::{
    zero_extended, LV1_ILLEGAL_PARAMETER_VALUE, LV1_NO_ENTRY, LV1_UNSUPPORTED_PARAMETER_VALUE,
};
use crate::lv1::table::{table_entry, TableEntry};

/// The r3 of a check whose high word holds `mark`.
fn marked(mark: u64, status: i64) -> u64 {
    (mark << 32) | zero_extended(status)
}

/// The row is the `LV1_CALL` row of Linux `lv1call.h` for 174.
#[test]
fn allocate_device_dma_region_carries_its_header_number_name_and_counts() {
    assert_eq!(ALLOCATE_DEVICE_DMA_REGION, 174);
    let row =
        call::lookup(ALLOCATE_DEVICE_DMA_REGION).map(|c| (c.name, c.inputs, c.outputs, c.source));
    assert_eq!(
        row,
        Some(("allocate_device_dma_region", 5, 1, CallSource::Linux))
    );
}

#[test]
fn the_4_93_handler_is_the_live_table_entry_of_174() {
    assert_eq!(DMA_REGION_HANDLERS.len(), 1);
    let handler = dma_region_handler("4.93").expect("the 4.93 handler");
    assert_eq!(handler.handler, 0x306A6C);
    assert_eq!(
        table_entry("4.93", ALLOCATE_DEVICE_DMA_REGION),
        Some(TableEntry::Handler(handler.handler))
    );
}

#[test]
fn no_dma_region_handler_row_exists_for_a_firmware_whose_lv1_is_not_read() {
    for firmware in ["3.70", "4.92", ""] {
        assert_eq!(dma_region_handler(firmware), None, "{firmware:?}");
    }
}

/// [OFW-4.93 lv1 a:0x306BE8-0x306BF4] `li 1; sldi 32; oris 0xffff; ori 0xffef`;
/// [OFW-4.93 lv1 a:0x306BFC-0x306C08] `li 16; sldi 32; oris 0xffff; ori 0xffef`;
/// [OFW-4.93 lv1 a:0x236420-0x236428] mark 1 and -6;
/// [OFW-4.93 lv1 a:0x236538] -11 with mark 0;
/// [OFW-4.93 lv1 a:0x237018-0x237024] `li 16; sldi 32; oris 0xffff; ori 0xfff1`;
/// [OFW-4.93 lv1 a:0x226928-0x22692C] mark 3 and -17;
/// [OFW-4.93 lv1 a:0x226964-0x226968] mark 4 and -17;
/// [OFW-4.93 lv1 a:0x22D4D0] -17, which the allocate function zero-extends
/// with mark 0. [OFW-4.93 lv1 a:0x2370B8-0x2370E4] sets bit (mark + 31).
#[test]
fn each_dma_region_check_r3_puts_its_mark_over_its_status() {
    let cases = [
        (
            DMA_REGION_BUS_RANGE_R3,
            marked(1, LV1_ILLEGAL_PARAMETER_VALUE),
        ),
        (
            DMA_REGION_FLAGS_RANGE_R3,
            marked(16, LV1_ILLEGAL_PARAMETER_VALUE),
        ),
        (DMA_REGION_NO_BUS_R3, marked(1, LV1_NO_ENTRY)),
        (DMA_REGION_NOT_OPEN_R3, 0x0000_0000_FFFF_FFF5),
        (
            DMA_REGION_FLAGS_R3,
            marked(16, LV1_UNSUPPORTED_PARAMETER_VALUE),
        ),
        (DMA_REGION_SIZE_R3, marked(4, LV1_ILLEGAL_PARAMETER_VALUE)),
        (
            DMA_REGION_PAGE_SHIFT_R3,
            marked(8, LV1_ILLEGAL_PARAMETER_VALUE),
        ),
        (DMA_REGION_TOO_LARGE_R3, 0x0000_0000_FFFF_FFEF),
    ];
    assert_eq!(cases.len(), 8);
    for (r3, expected) in cases {
        assert_eq!(r3, expected, "0x{expected:016X}");
    }
}

/// [OFW-4.93 lv1 a:0x22D404-0x22D41C] keeps a power of two;
/// [OFW-4.93 lv1 a:0x22D56C-0x22D570] doubles the highest set bit of
/// another size with `sldi r5, r5, 1`.
#[test]
fn the_rounded_size_is_the_next_power_of_two_and_wraps_above_bit_63() {
    let cases = [
        (0x1000, 0x1000),
        (0x10_0000, 0x10_0000),
        (0x3000, 0x4000),
        (0x1_0000_1000, 0x2_0000_0000),
        (1 << 63, 1 << 63),
        ((1 << 63) | 0x1000, 0),
        (u64::MAX, 0),
    ];
    assert_eq!(cases.len(), 7);
    for (size, rounded) in cases {
        assert_eq!(dma_region_rounded_size(size), rounded, "0x{size:X}");
    }
}

/// [OFW-4.93 lv1 a:0x226848-0x22688C] checks the size, the page shift and
/// the multiple in that order; [OFW-4.93 lv1 a:0x22D418-0x22D42C] and
/// [OFW-4.93 lv1 a:0x22D4B8-0x22D4D0] bound the rounded size at 2^32.
#[test]
fn the_shape_checks_run_in_lv1_order_and_pass_the_kernel_call() {
    let cases = [
        (0, 7, Some(DMA_REGION_SIZE_R3)),
        (0x1000, 13, Some(DMA_REGION_PAGE_SHIFT_R3)),
        (0x1000, (1 << 32) | 12, Some(DMA_REGION_PAGE_SHIFT_R3)),
        (0x1000, 64, Some(DMA_REGION_PAGE_SHIFT_R3)),
        (0x1000, u64::MAX, Some(DMA_REGION_PAGE_SHIFT_R3)),
        (0x1800, 12, Some(DMA_REGION_SIZE_R3)),
        (0x1_8000, 16, Some(DMA_REGION_SIZE_R3)),
        (0x10_0000, 24, Some(DMA_REGION_SIZE_R3)),
        (0x1_0000_1000, 24, Some(DMA_REGION_SIZE_R3)),
        (0x1_0000_0000, 24, None),
        (0x2_0000_0000, 24, Some(DMA_REGION_TOO_LARGE_R3)),
        (0x1_0100_0000, 24, Some(DMA_REGION_TOO_LARGE_R3)),
        (1 << 63, 24, Some(DMA_REGION_TOO_LARGE_R3)),
        ((1 << 63) | 0x100_0000, 24, None),
        (0x10_0000, 20, None),
    ];
    assert_eq!(cases.len(), 15);
    for (size, page_shift, r3) in cases {
        assert_eq!(
            dma_region_shape_r3(size, page_shift),
            r3,
            "size 0x{size:X} shift {page_shift}"
        );
    }
}

#[test]
fn only_the_captured_call_has_a_game_os_dma_region() {
    assert_eq!(GAME_OS_DMA_REGIONS.len(), 1);
    assert_eq!(
        game_os_dma_region("4.93", 1, 0, 0x10_0000, 20),
        Some(0xC000_0000)
    );
    let others = [
        ("4.92", 1, 0, 0x10_0000, 20),
        ("4.93", 2, 0, 0x10_0000, 20),
        ("4.93", 1, 1, 0x10_0000, 20),
        ("4.93", 1, 0, 0x20_0000, 20),
        ("4.93", 1, 0, 0x10_0000, 16),
    ];
    assert_eq!(others.len(), 5);
    for (firmware, bus, device, size, page_shift) in others {
        assert_eq!(
            game_os_dma_region(firmware, bus, device, size, page_shift),
            None,
            "{firmware} {bus} {device} 0x{size:X} {page_shift}"
        );
    }
}
