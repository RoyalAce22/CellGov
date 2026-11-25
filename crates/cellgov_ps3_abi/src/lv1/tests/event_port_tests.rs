use super::*;
use crate::lv1::call::{self, CallSource, CONSTRUCT_EVENT_RECEIVE_PORT};
use crate::lv1::table::{table_entry, TableEntry};

/// The row is the `LV1_CALL` row of Linux `lv1call.h` for 18.
#[test]
fn construct_event_receive_port_carries_its_header_number_name_and_counts() {
    assert_eq!(CONSTRUCT_EVENT_RECEIVE_PORT, 18);
    let row =
        call::lookup(CONSTRUCT_EVENT_RECEIVE_PORT).map(|c| (c.name, c.inputs, c.outputs, c.source));
    assert_eq!(
        row,
        Some(("construct_event_receive_port", 0, 1, CallSource::Linux))
    );
}

#[test]
fn the_4_93_handler_is_the_live_table_entry_of_18() {
    assert_eq!(EVENT_RECEIVE_PORT_HANDLERS.len(), 1);
    let handler = event_receive_port_handler("4.93").expect("the 4.93 handler");
    assert_eq!(handler.handler, 0x2EF424);
    assert_eq!(
        table_entry("4.93", CONSTRUCT_EVENT_RECEIVE_PORT),
        Some(TableEntry::Handler(handler.handler))
    );
}

#[test]
fn no_handler_row_or_port_id_exists_for_a_firmware_whose_lv1_is_not_read() {
    for firmware in ["3.70", "4.92", ""] {
        assert_eq!(event_receive_port_handler(firmware), None, "{firmware:?}");
        assert_eq!(game_os_port_calls(firmware), None, "{firmware:?}");
        assert_eq!(
            game_os_port_id(firmware, 0, RECORD_SC),
            None,
            "{firmware:?}"
        );
    }
}

#[test]
fn every_firmware_with_port_ids_has_a_read_handler() {
    assert_eq!(GAME_OS_PORT_CALLS.len(), 1);
    for calls in GAME_OS_PORT_CALLS {
        assert!(
            event_receive_port_handler(calls.firmware).is_some(),
            "{}",
            calls.firmware
        );
    }
}
