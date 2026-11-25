use super::*;
use crate::lv1::call::{self, CallSource, CONNECT_INTERRUPT_EVENT_RECEIVE_PORT};
use crate::lv1::status::{zero_extended, LV1_BUSY, LV1_ILLEGAL_PARAMETER_VALUE, LV1_NO_ENTRY};
use crate::lv1::table::{table_entry, TableEntry};

/// The r3 of a check whose high word holds `mark`.
fn marked(mark: u64, status: i64) -> u64 {
    (mark << 32) | zero_extended(status)
}

/// The row is the `LV1_CALL` row of Linux `lv1call.h` for 197.
#[test]
fn connect_interrupt_event_receive_port_carries_its_header_number_name_and_counts() {
    assert_eq!(CONNECT_INTERRUPT_EVENT_RECEIVE_PORT, 197);
    let row = call::lookup(CONNECT_INTERRUPT_EVENT_RECEIVE_PORT)
        .map(|c| (c.name, c.inputs, c.outputs, c.source));
    assert_eq!(
        row,
        Some((
            "connect_interrupt_event_receive_port",
            4,
            0,
            CallSource::Linux
        ))
    );
}

#[test]
fn the_4_93_handler_is_the_live_table_entry_of_197() {
    assert_eq!(INTERRUPT_PORT_HANDLERS.len(), 1);
    let handler = interrupt_port_handler("4.93").expect("the 4.93 handler");
    assert_eq!(handler.handler, 0x305CF0);
    assert_eq!(
        table_entry("4.93", CONNECT_INTERRUPT_EVENT_RECEIVE_PORT),
        Some(TableEntry::Handler(handler.handler))
    );
}

#[test]
fn no_interrupt_port_handler_row_exists_for_a_firmware_whose_lv1_is_not_read() {
    for firmware in ["3.70", "4.92", ""] {
        assert_eq!(interrupt_port_handler(firmware), None, "{firmware:?}");
    }
}

/// - [OFW-4.93 lv1 a:0x305E40-0x305E4C] `li 1; sldi 32; oris 0xffff; ori 0xffef`.
/// - [OFW-4.93 lv1 a:0x236420-0x236424] mark 1 and -6, which
///   [OFW-4.93 lv1 a:0x236AFC-0x236B18] puts in the high word.
/// - [OFW-4.93 lv1 a:0x236538] -11 with mark 0.
/// - [OFW-4.93 lv1 a:0x236AB4-0x236AE0] `li 8; sldi 32; oris 0xffff; ori 0xfffa`.
/// - [OFW-4.93 lv1 a:0x236BD8-0x236BE4] `li 8; sldi 32; oris 0xffff; ori 0xfff7`.
/// - [OFW-4.93 lv1 a:0x236BEC-0x236BF8] `li 4; sldi 32; oris 0xffff; ori 0xfffa`.
#[test]
fn each_interrupt_port_check_r3_puts_its_mark_over_its_status() {
    let cases = [
        (
            INTERRUPT_PORT_BUS_RANGE_R3,
            marked(1, LV1_ILLEGAL_PARAMETER_VALUE),
        ),
        (INTERRUPT_PORT_NO_BUS_R3, marked(1, LV1_NO_ENTRY)),
        (INTERRUPT_PORT_NOT_OPEN_R3, 0x0000_0000_FFFF_FFF5),
        (INTERRUPT_PORT_R6_R3, marked(8, LV1_NO_ENTRY)),
        (INTERRUPT_PORT_BUSY_R3, marked(8, LV1_BUSY)),
        (INTERRUPT_PORT_NO_PORT_R3, marked(4, LV1_NO_ENTRY)),
    ];
    assert_eq!(cases.len(), 6);
    for (r3, expected) in cases {
        assert_eq!(r3, expected, "0x{expected:016X}");
    }
}
