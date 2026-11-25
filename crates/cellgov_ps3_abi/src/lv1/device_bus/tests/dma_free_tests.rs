use super::*;
use crate::lv1::call::{self, CallSource, FREE_DEVICE_DMA_REGION};
use crate::lv1::status::{zero_extended, LV1_BUSY, LV1_ILLEGAL_PARAMETER_VALUE, LV1_NO_ENTRY};
use crate::lv1::table::{table_entry, TableEntry};

/// The r3 of a check that stores `mark` (1 or more): the free function
/// zero-extends the status and sets bit (mark + 31).
fn marked(mark: u32, status: i64) -> u64 {
    (1u64 << (mark + 31)) | zero_extended(status)
}

/// The row is the `LV1_CALL` row of Linux `lv1call.h` for 175.
#[test]
fn free_device_dma_region_carries_its_header_number_name_and_counts() {
    assert_eq!(FREE_DEVICE_DMA_REGION, 175);
    let row = call::lookup(FREE_DEVICE_DMA_REGION).map(|c| (c.name, c.inputs, c.outputs, c.source));
    assert_eq!(
        row,
        Some(("free_device_dma_region", 3, 0, CallSource::Linux))
    );
}

#[test]
fn the_4_93_handler_is_the_live_table_entry_of_175() {
    assert_eq!(DMA_FREE_HANDLERS.len(), 1);
    let handler = dma_free_handler("4.93").expect("the 4.93 handler");
    assert_eq!(handler.handler, 0x306918);
    assert_eq!(
        table_entry("4.93", FREE_DEVICE_DMA_REGION),
        Some(TableEntry::Handler(handler.handler))
    );
    for firmware in ["3.70", "4.92", ""] {
        assert_eq!(dma_free_handler(firmware), None, "{firmware:?}");
    }
}

/// [OFW-4.93 lv1 a:0x306A58-0x306A64] `li 1; sldi 32; oris 0xffff; ori 0xffef`;
/// [OFW-4.93 lv1 a:0x236420-0x236428] mark 1 and -6;
/// [OFW-4.93 lv1 a:0x236538] -11 with mark 0;
/// [OFW-4.93 lv1 a:0x2263BC-0x2263C4] mark 3 and -6;
/// [OFW-4.93 lv1 a:0x22635C-0x226364] mark 3 and -17;
/// [OFW-4.93 lv1 a:0x2276BC] -9, with mark 3 [OFW-4.93 lv1 a:0x22631C-0x226320];
/// [OFW-4.93 lv1 a:0x236ED8-0x236EE4] sets bit (mark + 31).
#[test]
fn each_dma_free_check_r3_puts_its_mark_over_its_status() {
    let cases = [
        (
            DMA_FREE_BUS_RANGE_R3,
            marked(1, LV1_ILLEGAL_PARAMETER_VALUE),
        ),
        (DMA_FREE_NO_BUS_R3, marked(1, LV1_NO_ENTRY)),
        (DMA_FREE_NOT_OPEN_R3, 0x0000_0000_FFFF_FFF5),
        (DMA_FREE_NO_REGION_R3, marked(3, LV1_NO_ENTRY)),
        (DMA_FREE_ADDRESS_R3, marked(3, LV1_ILLEGAL_PARAMETER_VALUE)),
        (DMA_FREE_BUSY_R3, marked(3, LV1_BUSY)),
    ];
    assert_eq!(cases.len(), 6);
    for (r3, expected) in cases {
        assert_eq!(r3, expected, "{r3:#x}");
    }
}
