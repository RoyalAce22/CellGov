use super::*;
use crate::lv1::call::{self, CallSource, STORAGE_GET_ASYNC_STATUS};
use crate::lv1::table::{table_entry, TableEntry};

/// The row is the `LV1_CALL` row of Linux `lv1call.h` for 249.
#[test]
fn storage_get_async_status_carries_its_header_number_name_and_counts() {
    assert_eq!(STORAGE_GET_ASYNC_STATUS, 249);
    let row =
        call::lookup(STORAGE_GET_ASYNC_STATUS).map(|c| (c.name, c.inputs, c.outputs, c.source));
    assert_eq!(
        row,
        Some(("storage_get_async_status", 1, 2, CallSource::Linux))
    );
}

#[test]
fn the_4_93_handler_is_the_live_table_entry_of_249() {
    assert_eq!(STORAGE_STATUS_HANDLERS.len(), 1);
    let handler = storage_status_handler("4.93").expect("the 4.93 handler");
    assert_eq!(handler.handler, 0x30_93AC);
    assert_eq!(
        table_entry("4.93", STORAGE_GET_ASYNC_STATUS),
        Some(TableEntry::Handler(handler.handler))
    );
}

#[test]
fn no_storage_status_handler_row_exists_for_a_firmware_whose_lv1_is_not_read() {
    for firmware in ["3.70", "4.92", ""] {
        assert_eq!(storage_status_handler(firmware), None, "{firmware:?}");
    }
}

/// [OFW-4.93 lv1 a:0x251F58] `li r0, -6`; [OFW-4.93 lv1 a:0x251F28]
/// `li r0, -11`; [OFW-4.93 lv1 a:0x25600C] `li r3, -6`.
#[test]
fn the_status_core_and_method_statuses_keep_their_linux_values() {
    assert_eq!(ASYNC_STATUS_NO_DEVICE_STATUS, -6);
    assert_eq!(ASYNC_STATUS_NOT_READY_STATUS, -11);
    assert_eq!(ASYNC_STATUS_NONE_STATUS, -6);
}

/// [OFW-4.93 lv1 a:0x256060-0x256068] `ld r0, 2616(r31); cmpdi; bge`:
/// only a negative r5 reaches the slot word.
#[test]
fn the_slot_word_flag_is_the_sign_bit_alone() {
    assert_eq!(ASYNC_STATUS_SLOT_WORD_FLAG.count_ones(), 1);
    assert!((ASYNC_STATUS_SLOT_WORD_FLAG as i64) < 0);
    assert!((!ASYNC_STATUS_SLOT_WORD_FLAG as i64) >= 0);
}
