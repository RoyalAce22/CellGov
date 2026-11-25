use super::*;
use crate::lv1::call::{self, CallSource, PAUSE};
use crate::lv1::status::{zero_extended, LV1_ILLEGAL_PARAMETER_VALUE};
use crate::lv1::table::{table_entry, TableEntry};

/// The row is the `LV1_CALL` row of Linux `lv1call.h` for 9.
#[test]
fn pause_carries_its_header_number_name_and_counts() {
    assert_eq!(PAUSE, 9);
    let row = call::lookup(PAUSE).map(|c| (c.name, c.inputs, c.outputs, c.source));
    assert_eq!(row, Some(("pause", 1, 0, CallSource::Linux)));
}

#[test]
fn the_4_93_handler_is_the_live_table_entry_of_9() {
    assert_eq!(PAUSE_HANDLERS.len(), 1);
    let handler = pause_handler("4.93").expect("the 4.93 handler");
    assert_eq!(handler.handler, 0x2EB618);
    assert_eq!(
        table_entry("4.93", PAUSE),
        Some(TableEntry::Handler(handler.handler))
    );
}

#[test]
fn no_pause_handler_row_exists_for_a_firmware_whose_lv1_is_not_read() {
    for firmware in ["3.70", "4.92", ""] {
        assert_eq!(pause_handler(firmware), None, "{firmware:?}");
    }
}

/// [OFW-4.93 lv1 a:0x2EB784-0x2EB794] `li 1; sldi 32; oris 0xffff; ori 0xffef`.
#[test]
fn a_failed_mode_check_returns_illegal_parameter_value_with_r3_marked() {
    assert_eq!(
        PAUSE_MODE_R3,
        (1u64 << 32) | zero_extended(LV1_ILLEGAL_PARAMETER_VALUE)
    );
}

#[test]
fn the_two_modes_are_the_values_that_equal_their_own_bit_63() {
    let modes = [PAUSE_MODE_DECREMENTER_WAKES, PAUSE_MODE_DECREMENTER_IGNORED];
    assert_eq!(modes, [0, 1]);
    assert!(modes.iter().all(|&mode| mode == mode & 1));
}
