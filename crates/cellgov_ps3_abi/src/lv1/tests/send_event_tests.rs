use super::*;
use crate::lv1::call::{self, CallSource, SEND_EVENT_LOCALLY};
use crate::lv1::status::{zero_extended, LV1_NO_ENTRY};
use crate::lv1::table::{table_entry, TableEntry};

/// The row is the `LV1_CALL` row of Linux `lv1call.h` for 24.
#[test]
fn send_event_locally_carries_its_header_number_name_and_counts() {
    assert_eq!(SEND_EVENT_LOCALLY, 24);
    let row = call::lookup(SEND_EVENT_LOCALLY).map(|c| (c.name, c.inputs, c.outputs, c.source));
    assert_eq!(row, Some(("send_event_locally", 1, 0, CallSource::Linux)));
}

#[test]
fn the_4_93_handler_is_the_live_table_entry_of_24() {
    assert_eq!(SEND_EVENT_HANDLERS.len(), 1);
    let handler = send_event_handler("4.93").expect("the 4.93 handler");
    assert_eq!(handler.handler, 0x2EF530);
    assert_eq!(
        table_entry("4.93", SEND_EVENT_LOCALLY),
        Some(TableEntry::Handler(handler.handler))
    );
}

#[test]
fn no_send_event_handler_row_exists_for_a_firmware_whose_lv1_is_not_read() {
    for firmware in ["3.70", "4.92", ""] {
        assert_eq!(send_event_handler(firmware), None, "{firmware:?}");
    }
}

/// [OFW-4.93 lv1 a:0x2E27EC-0x2E27F8] `li 1; sldi 32; oris 0xffff; ori 0xfffa`.
#[test]
fn a_failed_lookup_returns_no_entry_with_r3_marked() {
    assert_eq!(
        SEND_EVENT_NO_ENTRY_R3,
        (1u64 << 32) | zero_extended(LV1_NO_ENTRY)
    );
}
