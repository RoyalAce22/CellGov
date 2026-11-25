use super::*;
use crate::lv1::call::{self, CallSource, STORAGE_READ};
use crate::lv1::table::{table_entry, TableEntry};

/// The row is the `LV1_CALL` row of Linux `lv1call.h` for 245.
#[test]
fn storage_read_carries_its_header_number_name_and_counts() {
    assert_eq!(STORAGE_READ, 245);
    let row = call::lookup(STORAGE_READ).map(|c| (c.name, c.inputs, c.outputs, c.source));
    assert_eq!(row, Some(("storage_read", 6, 1, CallSource::Linux)));
}

#[test]
fn the_4_93_handler_is_the_live_table_entry_of_245() {
    assert_eq!(STORAGE_READ_HANDLERS.len(), 1);
    let handler = storage_read_handler("4.93").expect("the 4.93 handler");
    assert_eq!(handler.handler, 0x30_8B88);
    assert_eq!(
        table_entry("4.93", STORAGE_READ),
        Some(TableEntry::Handler(handler.handler))
    );
}

#[test]
fn no_storage_read_handler_row_exists_for_a_firmware_whose_lv1_is_not_read() {
    for firmware in ["3.70", "4.92", ""] {
        assert_eq!(storage_read_handler(firmware), None, "{firmware:?}");
    }
}

/// [OFW-4.93 lv1 a:0x2515C0] `li r0, -6`; [OFW-4.93 lv1 a:0x251580]
/// `li r0, -11`; [OFW-4.93 lv1 a:0x2593DC] `li r3, -17`;
/// [OFW-4.93 lv1 a:0x259394] `li r3, -6`.
#[test]
fn the_read_core_and_method_statuses_keep_their_lv1_values() {
    assert_eq!(STORAGE_READ_NO_DEVICE_STATUS, -6);
    assert_eq!(STORAGE_READ_NOT_READY_STATUS, -11);
    assert_eq!(NOTIFY_READ_BAD_REQUEST_STATUS, -17);
    assert_eq!(NOTIFY_READ_NO_RECORD_STATUS, -6);
}

/// [OFW-4.93 lv1 a:0x25946C] `clrlwi r0, r11, 31`;
/// [OFW-4.93 lv1 a:0x259524] `rlwinm r0, r11, 0, 30, 30`;
/// [OFW-4.93 lv1 a:0x2595EC] `rlwinm r0, r11, 0, 29, 29`.
#[test]
fn the_three_event_mask_bits_are_bits_0_1_and_2() {
    assert_eq!(
        [
            NOTIFY_READ_TYPE0_EVENT_BIT,
            NOTIFY_READ_DEVICE_EVENT_BIT,
            NOTIFY_READ_QUEUED_EVENT_BIT,
        ],
        [1, 2, 4]
    );
}
