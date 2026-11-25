use super::*;
use crate::lv1::call::{self, CallSource, STORAGE_WRITE};
use crate::lv1::table::{table_entry, TableEntry};

/// The row is the `LV1_CALL` row of Linux `lv1call.h` for 246.
#[test]
fn storage_write_carries_its_header_number_name_and_counts() {
    assert_eq!(STORAGE_WRITE, 246);
    let row = call::lookup(STORAGE_WRITE).map(|c| (c.name, c.inputs, c.outputs, c.source));
    assert_eq!(row, Some(("storage_write", 6, 1, CallSource::Linux)));
}

#[test]
fn the_4_93_handler_is_the_live_table_entry_of_246() {
    assert_eq!(STORAGE_WRITE_HANDLERS.len(), 1);
    let handler = storage_write_handler("4.93").expect("the 4.93 handler");
    assert_eq!(handler.handler, 0x30_8A08);
    assert_eq!(
        table_entry("4.93", STORAGE_WRITE),
        Some(TableEntry::Handler(handler.handler))
    );
}

#[test]
fn no_storage_write_handler_row_exists_for_a_firmware_whose_lv1_is_not_read() {
    for firmware in ["3.70", "4.92", ""] {
        assert_eq!(storage_write_handler(firmware), None, "{firmware:?}");
    }
}

/// [OFW-4.93 lv1 a:0x25A6B8] `li r31, -17`; [OFW-4.93 lv1 a:0x2539C8]
/// `li r10, -9`; [OFW-4.93 lv1 a:0x254500] `li r3, -3`.
#[test]
fn the_notification_write_statuses_keep_their_linux_values() {
    assert_eq!(NOTIFY_WRITE_BAD_REQUEST_STATUS, -17);
    assert_eq!(NOTIFY_REQUESTS_FULL_STATUS, -9);
    assert_eq!(NOTIFY_WRITE_BUFFER_PAST_REGION_STATUS, -3);
}

/// [OFW-4.93 lv1 a:0x25398C-0x2539A0] `li -1; clrldi 48; cmpd`: the
/// bitmap is full at 0xFFFF.
#[test]
fn the_request_bitmap_holds_one_bit_for_each_tag() {
    assert_eq!(0xFFFF_u64.count_ones(), u32::from(NOTIFY_REQUEST_TAGS));
}
