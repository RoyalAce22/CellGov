use super::*;
use crate::lv1::call::{self, CallSource, GPU_OPEN};
use crate::lv1::table::{table_entry, TableEntry};

/// The row is the `LV1_CALL` row of Linux `lv1call.h` for 210.
#[test]
fn gpu_open_carries_its_header_number_name_and_counts() {
    assert_eq!(GPU_OPEN, 210);
    let row = call::lookup(GPU_OPEN).map(|c| (c.name, c.inputs, c.outputs, c.source));
    assert_eq!(row, Some(("gpu_open", 1, 0, CallSource::Linux)));
}

#[test]
fn the_4_93_handler_is_the_live_table_entry_of_210() {
    assert_eq!(GPU_OPEN_HANDLERS.len(), 1);
    let handler = gpu_open_handler("4.93").expect("the 4.93 handler");
    assert_eq!(handler.handler, 0x20E908);
    assert_eq!(
        table_entry("4.93", GPU_OPEN),
        Some(TableEntry::Handler(handler.handler))
    );
    for firmware in ["3.70", "4.92", ""] {
        assert_eq!(gpu_open_handler(firmware), None, "{firmware:?}");
    }
}

/// [OFW-4.93 lv1 a:0x20EC24-0x20EC30] `oris 0xffff; ori 0xffef` on 0;
/// [OFW-4.93 lv1 a:0x20C69C-0x20C6A4] `li 0; oris 0xffff; ori 0xfffa`;
/// [OFW-4.93 lv1 a:0x20C68C-0x20C694] `li 0; oris 0xffff; ori 0xfff7`;
/// [OFW-4.93 lv1 a:0x20C5E0-0x20C5E8] `li 0; oris 0xffff; ori 0xffef`.
#[test]
fn each_gpu_open_check_r3_is_its_status_zero_extended() {
    let cases = [
        (GPU_OPEN_BOOT_MODE_R3, 0x0000_0000_FFFF_FFEF),
        (GPU_OPEN_DISABLED_R3, 0x0000_0000_FFFF_FFFA),
        (GPU_OPEN_BUSY_R3, 0x0000_0000_FFFF_FFF7),
        (GPU_OPEN_SLOT_R3, 0x0000_0000_FFFF_FFEF),
    ];
    assert_eq!(cases.len(), 4);
    for (r3, expected) in cases {
        assert_eq!(r3, expected, "{r3:#x}");
    }
}

/// [OFW-4.93 lv1 a:0x20E964] `cmpldi 3`; [OFW-4.93 lv1 a:0x20C590]
/// `cmpldi 1` with `ble`; [OFW-4.93 lv1 a:0x20C5B8] `cmpwi 255`;
/// [OFW-4.93 lv1 a:0x20C5C0] `cmpldi 0xf` with `bgt`.
#[test]
fn the_gpu_open_bounds_are_the_handler_compares() {
    assert_eq!(GPU_OPEN_BOOT_MODE_MAX, 3);
    assert_eq!(GPU_OPEN_RSX_ENABLE_MIN, 2);
    assert_eq!(GPU_NO_OWNER, 0xFF);
    assert_eq!(GPU_SLOTS, 16);
}
