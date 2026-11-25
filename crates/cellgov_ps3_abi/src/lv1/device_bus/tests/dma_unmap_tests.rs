use super::*;
use crate::lv1::call::{self, CallSource, UNMAP_DEVICE_DMA_REGION};
use crate::lv1::status::{zero_extended, LV1_ILLEGAL_PARAMETER_VALUE, LV1_NO_ENTRY};
use crate::lv1::table::{table_entry, TableEntry};

/// The r3 of a check that stores `mark` (1 or more): the unmap function
/// zero-extends the status and sets bit (mark + 31).
fn marked(mark: u32, status: i64) -> u64 {
    (1u64 << (mark + 31)) | zero_extended(status)
}

/// The row is the `LV1_CALL` row of Linux `lv1call.h` for 177.
#[test]
fn unmap_device_dma_region_carries_its_header_number_name_and_counts() {
    assert_eq!(UNMAP_DEVICE_DMA_REGION, 177);
    let row =
        call::lookup(UNMAP_DEVICE_DMA_REGION).map(|c| (c.name, c.inputs, c.outputs, c.source));
    assert_eq!(
        row,
        Some(("unmap_device_dma_region", 4, 0, CallSource::Linux))
    );
}

#[test]
fn the_4_93_handler_is_the_live_table_entry_of_177() {
    assert_eq!(DMA_UNMAP_HANDLERS.len(), 1);
    let handler = dma_unmap_handler("4.93").expect("the 4.93 handler");
    assert_eq!(handler.handler, 0x30660C);
    assert_eq!(
        table_entry("4.93", UNMAP_DEVICE_DMA_REGION),
        Some(TableEntry::Handler(handler.handler))
    );
    for firmware in ["3.70", "4.92", ""] {
        assert_eq!(dma_unmap_handler(firmware), None, "{firmware:?}");
    }
}

/// [OFW-4.93 lv1 a:0x30675C-0x306768] `li 1; sldi 32; oris 0xffff; ori 0xffef`;
/// [OFW-4.93 lv1 a:0x236420-0x236428] mark 1 and -6;
/// [OFW-4.93 lv1 a:0x236538] -11 with mark 0;
/// [OFW-4.93 lv1 a:0x226680-0x226684] mark 3 and -6;
/// [OFW-4.93 lv1 a:0x226644-0x226648] mark 3 and -17;
/// [OFW-4.93 lv1 a:0x226634-0x226638] mark 4 and -17;
/// [OFW-4.93 lv1 a:0x236CB4-0x236CCC] sets bit (mark + 31).
#[test]
fn each_dma_unmap_check_r3_puts_its_mark_over_its_status() {
    let cases = [
        (
            DMA_UNMAP_BUS_RANGE_R3,
            marked(1, LV1_ILLEGAL_PARAMETER_VALUE),
        ),
        (DMA_UNMAP_NO_BUS_R3, marked(1, LV1_NO_ENTRY)),
        (DMA_UNMAP_NOT_OPEN_R3, 0x0000_0000_FFFF_FFF5),
        (DMA_UNMAP_NO_MAPPING_R3, marked(3, LV1_NO_ENTRY)),
        (DMA_UNMAP_ADDRESS_R3, marked(3, LV1_ILLEGAL_PARAMETER_VALUE)),
        (DMA_UNMAP_LENGTH_R3, marked(4, LV1_ILLEGAL_PARAMETER_VALUE)),
    ];
    assert_eq!(cases.len(), 6);
    for (r3, expected) in cases {
        assert_eq!(r3, expected, "{r3:#x}");
    }
}
