use super::*;
use crate::lv1::call::{self, CallSource, STORAGE_SEND_DEVICE_COMMAND};
use crate::lv1::table::{table_entry, TableEntry};

/// The row is the `LV1_CALL` row of Linux `lv1call.h` for 248.
#[test]
fn storage_send_device_command_carries_its_header_number_name_and_counts() {
    assert_eq!(STORAGE_SEND_DEVICE_COMMAND, 248);
    let row =
        call::lookup(STORAGE_SEND_DEVICE_COMMAND).map(|c| (c.name, c.inputs, c.outputs, c.source));
    assert_eq!(
        row,
        Some(("storage_send_device_command", 6, 1, CallSource::Linux))
    );
}

#[test]
fn the_4_93_handler_is_the_live_table_entry_of_248() {
    assert_eq!(STORAGE_COMMAND_HANDLERS.len(), 1);
    let handler = storage_command_handler("4.93").expect("the 4.93 handler");
    assert_eq!(handler.handler, 0x30_94E4);
    assert_eq!(
        table_entry("4.93", STORAGE_SEND_DEVICE_COMMAND),
        Some(TableEntry::Handler(handler.handler))
    );
    assert_eq!(storage_command_handler("4.92"), None);
}

/// [OFW-4.93 lv1 a:0x272C84-0x272CA8] builds the mask
/// 0x0001_02D2_1E1B_0006, and [OFW-4.93 lv1 a:0x272C7C] refuses an id
/// above 0x30.
#[test]
fn the_dispatcher_accepts_exactly_the_ids_of_its_mask() {
    let accepted: Vec<u32> = (0..=0xFF).filter(|&id| command_accepted(id)).collect();
    assert_eq!(
        accepted,
        vec![1, 2, 16, 17, 19, 20, 25, 26, 27, 28, 33, 36, 38, 39, 41, 48]
    );
    assert!(accepted.contains(&CONTROLLER_QUERY_COMMAND));
    assert!(!command_accepted(u32::MAX));
}
