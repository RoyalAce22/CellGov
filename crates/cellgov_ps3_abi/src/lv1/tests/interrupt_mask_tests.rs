use super::*;
use crate::lv1::call::{self, CallSource, DID_UPDATE_INTERRUPT_MASK};
use crate::lv1::status::{zero_extended, LV1_NO_ENTRY};
use crate::lv1::table::{table_entry, TableEntry};

/// The row is the `LV1_CALL` row of Linux `lv1call.h` for 31.
#[test]
fn did_update_interrupt_mask_carries_its_header_number_name_and_counts() {
    assert_eq!(DID_UPDATE_INTERRUPT_MASK, 31);
    let row =
        call::lookup(DID_UPDATE_INTERRUPT_MASK).map(|c| (c.name, c.inputs, c.outputs, c.source));
    assert_eq!(
        row,
        Some(("did_update_interrupt_mask", 2, 0, CallSource::Linux))
    );
}

#[test]
fn the_4_93_handler_is_the_live_table_entry_of_31() {
    assert_eq!(INTERRUPT_MASK_HANDLERS.len(), 1);
    let handler = interrupt_mask_handler("4.93").expect("the 4.93 handler");
    assert_eq!(handler.handler, 0x2E9354);
    assert_eq!(
        table_entry("4.93", DID_UPDATE_INTERRUPT_MASK),
        Some(TableEntry::Handler(handler.handler))
    );
}

#[test]
fn no_handler_row_exists_for_a_firmware_whose_lv1_is_not_read() {
    for firmware in ["3.70", "4.92", ""] {
        assert_eq!(interrupt_mask_handler(firmware), None, "{firmware:?}");
    }
}

#[test]
fn each_refusal_marks_the_argument_it_names() {
    let marked = |register: u32, status: i64| (1u64 << (32 + register - 3)) | zero_extended(status);
    assert_eq!(INTERRUPT_MASK_PPE_ID_R3, marked(3, LV1_NO_ENTRY));
    assert_eq!(INTERRUPT_MASK_THREAD_R3, marked(4, LV1_NO_ENTRY));
}
