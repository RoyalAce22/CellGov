use super::*;
use crate::lv1::call::{self, CallSource, UNMAP_DEVICE_MMIO_REGION};
use crate::lv1::status::{zero_extended, LV1_ILLEGAL_PARAMETER_VALUE, LV1_NO_ENTRY};
use crate::lv1::table::{table_entry, TableEntry};

/// The r3 of a check whose high word holds `bits`.
fn marked(bits: u64, status: i64) -> u64 {
    (bits << 32) | zero_extended(status)
}

/// The row is the `LV1_CALL` row of Linux `lv1call.h` for 173.
#[test]
fn unmap_device_mmio_region_carries_its_header_number_name_and_counts() {
    assert_eq!(UNMAP_DEVICE_MMIO_REGION, 173);
    let row =
        call::lookup(UNMAP_DEVICE_MMIO_REGION).map(|c| (c.name, c.inputs, c.outputs, c.source));
    assert_eq!(
        row,
        Some(("unmap_device_mmio_region", 3, 0, CallSource::Linux))
    );
}

#[test]
fn the_4_93_handler_is_the_live_table_entry_of_173() {
    assert_eq!(MMIO_UNMAP_HANDLERS.len(), 1);
    let handler = mmio_unmap_handler("4.93").expect("the 4.93 handler");
    assert_eq!(handler.handler, 0x306C10);
    assert_eq!(
        table_entry("4.93", UNMAP_DEVICE_MMIO_REGION),
        Some(TableEntry::Handler(handler.handler))
    );
    for firmware in ["3.70", "4.92", ""] {
        assert_eq!(mmio_unmap_handler(firmware), None, "{firmware:?}");
    }
}

/// [OFW-4.93 lv1 a:0x306D50-0x306D5C] `li 1; sldi 32; oris 0xffff; ori 0xffef`;
/// [OFW-4.93 lv1 a:0x236420-0x236428] mark 1 and -6;
/// [OFW-4.93 lv1 a:0x236538] -11 with mark 0;
/// [OFW-4.93 lv1 a:0x237240-0x23724C] `li 4; sldi 32; oris 0xffff; ori 0xffef`;
/// [OFW-4.93 lv1 a:0x228B58-0x228B5C] mark 3 and -6;
/// [OFW-4.93 lv1 a:0x237200-0x23720C] sets bit (mark + 31).
#[test]
fn each_mmio_unmap_check_r3_puts_its_mark_over_its_status() {
    let cases = [
        (
            MMIO_UNMAP_BUS_RANGE_R3,
            marked(1, LV1_ILLEGAL_PARAMETER_VALUE),
        ),
        (MMIO_UNMAP_NO_BUS_R3, marked(1, LV1_NO_ENTRY)),
        (MMIO_UNMAP_NOT_OPEN_R3, 0x0000_0000_FFFF_FFF5),
        (
            MMIO_UNMAP_DEVICE_0_R3,
            marked(4, LV1_ILLEGAL_PARAMETER_VALUE),
        ),
        (MMIO_UNMAP_NO_MAPPING_R3, marked(4, LV1_NO_ENTRY)),
    ];
    assert_eq!(cases.len(), 5);
    for (r3, expected) in cases {
        assert_eq!(r3, expected, "{r3:#x}");
    }
}
