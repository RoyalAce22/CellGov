use super::*;
use crate::lv1::call::{self, CallSource, END_OF_INTERRUPT_EXT};
use crate::lv1::status::{zero_extended, LV1_NO_ENTRY, LV1_WRONG_STATE};
use crate::lv1::table::{table_entry, TableEntry};

/// The row is the `LV1_CALL` row of Linux `lv1call.h` for 30.
#[test]
fn end_of_interrupt_ext_carries_its_header_number_name_and_counts() {
    assert_eq!(END_OF_INTERRUPT_EXT, 30);
    let row = call::lookup(END_OF_INTERRUPT_EXT).map(|c| (c.name, c.inputs, c.outputs, c.source));
    assert_eq!(row, Some(("end_of_interrupt_ext", 3, 0, CallSource::Linux)));
}

#[test]
fn the_4_93_handler_is_the_live_table_entry_of_30() {
    assert_eq!(END_OF_INTERRUPT_HANDLERS.len(), 1);
    let handler = end_of_interrupt_handler("4.93").expect("the 4.93 handler");
    assert_eq!(handler.handler, 0x2E9710);
    assert_eq!(
        table_entry("4.93", END_OF_INTERRUPT_EXT),
        Some(TableEntry::Handler(handler.handler))
    );
}

#[test]
fn no_end_of_interrupt_handler_row_exists_for_a_firmware_whose_lv1_is_not_read() {
    for firmware in ["3.70", "4.92", ""] {
        assert_eq!(end_of_interrupt_handler(firmware), None, "{firmware:?}");
    }
}

/// [OFW-4.93 lv1 a:0x2E98CC-0x2E98D8] [OFW-4.93 lv1 a:0x2E9A18-0x2E9A28]
/// [OFW-4.93 lv1 a:0x2E9A60-0x2E9A70] [OFW-4.93 lv1 a:0x2E9A2C-0x2E9A3C]
/// `li N; sldi 32; oris 0xffff; ori 0xfffa` or `ori 0xfff5`.
#[test]
fn each_marked_r3_marks_the_argument_it_names() {
    let marked = |register: u32, status: i64| (1u64 << (32 + register - 3)) | zero_extended(status);
    assert_eq!(END_OF_INTERRUPT_PPE_ID_R3, marked(3, LV1_NO_ENTRY));
    assert_eq!(END_OF_INTERRUPT_THREAD_R3, marked(4, LV1_NO_ENTRY));
    assert_eq!(END_OF_INTERRUPT_PLUG_R3, marked(5, LV1_NO_ENTRY));
    assert_eq!(END_OF_INTERRUPT_WRONG_STATE_R3, marked(5, LV1_WRONG_STATE));
}

/// [OFW-4.93 lv1 a:0x2E9A50-0x2E9A58] `li 0; oris 0xffff; ori 0xfff5`.
#[test]
fn a_plug_with_no_pending_raise_gets_wrong_state_with_no_mark() {
    assert_eq!(
        END_OF_INTERRUPT_NOT_PENDING_R3,
        zero_extended(LV1_WRONG_STATE)
    );
}
