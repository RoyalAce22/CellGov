use super::*;
use crate::lv1::call::{self, CallSource, NET_SET_INTERRUPT_STATUS_INDICATOR};
use crate::lv1::status::{
    zero_extended, LV1_DUPLICATE_ENTRY, LV1_ILLEGAL_PARAMETER_VALUE, LV1_NO_ENTRY,
};
use crate::lv1::table::{table_entry, TableEntry};

/// The r3 of a check whose high word holds `mark`.
fn marked(mark: u64, status: i64) -> u64 {
    (mark << 32) | zero_extended(status)
}

/// The r3 that `li li; sldi 32; oris oris; ori ori` builds.
fn built(li: u64, oris: u64, ori: u64) -> u64 {
    (li << 32) | (oris << 16) | ori
}

/// The row is the `LV1_CALL` row of Linux `lv1call.h` for 191.
#[test]
fn net_set_interrupt_status_indicator_carries_its_header_number_name_and_counts() {
    assert_eq!(NET_SET_INTERRUPT_STATUS_INDICATOR, 191);
    let row = call::lookup(NET_SET_INTERRUPT_STATUS_INDICATOR)
        .map(|c| (c.name, c.inputs, c.outputs, c.source));
    assert_eq!(
        row,
        Some((
            "net_set_interrupt_status_indicator",
            4,
            0,
            CallSource::Linux
        ))
    );
}

#[test]
fn the_4_93_handler_is_the_live_table_entry_of_191() {
    assert_eq!(STATUS_INDICATOR_HANDLERS.len(), 1);
    let handler = status_indicator_handler("4.93").expect("the 4.93 handler");
    assert_eq!(handler.handler, 0x308204);
    assert_eq!(
        table_entry("4.93", NET_SET_INTERRUPT_STATUS_INDICATOR),
        Some(TableEntry::Handler(handler.handler))
    );
}

#[test]
fn no_status_indicator_handler_row_exists_for_a_firmware_whose_lv1_is_not_read() {
    for firmware in ["3.70", "4.92", ""] {
        assert_eq!(status_indicator_handler(firmware), None, "{firmware:?}");
    }
}

/// - [OFW-4.93 lv1 a:0x3084B0-0x3084BC] `li 1; sldi 32; oris 0xffff; ori 0xffef`.
/// - [OFW-4.93 lv1 a:0x3083E8-0x3083F4] `li 1; sldi 32; oris 0xffff; ori 0xfffa`.
/// - [OFW-4.93 lv1 a:0x308334-0x308340] `li 2; sldi 32; oris 0xffff; ori 0xfffa`.
/// - [OFW-4.93 lv1 a:0x3083D4-0x3083E0] `li 8; sldi 32; oris 0xffff; ori 0xffef`.
/// - [OFW-4.93 lv1 a:0x308408-0x308414] `li 4; sldi 32; oris 0xffff; ori 0xffef`.
/// - [OFW-4.93 lv1 a:0x3084A0-0x3084A8] `li 0; oris 0xffff; ori 0xfff9`.
#[test]
fn each_check_r3_marks_the_argument_it_reads_over_its_status() {
    let cases = [
        (
            STATUS_INDICATOR_BUS_RANGE_R3,
            marked(1, LV1_ILLEGAL_PARAMETER_VALUE),
            built(1, 0xffff, 0xffef),
        ),
        (
            STATUS_INDICATOR_NO_BUS_R3,
            marked(1, LV1_NO_ENTRY),
            built(1, 0xffff, 0xfffa),
        ),
        (
            STATUS_INDICATOR_NO_DEVICE_R3,
            marked(2, LV1_NO_ENTRY),
            built(2, 0xffff, 0xfffa),
        ),
        (
            STATUS_INDICATOR_R6_R3,
            marked(8, LV1_ILLEGAL_PARAMETER_VALUE),
            built(8, 0xffff, 0xffef),
        ),
        (
            STATUS_INDICATOR_ADDRESS_R3,
            marked(4, LV1_ILLEGAL_PARAMETER_VALUE),
            built(4, 0xffff, 0xffef),
        ),
        (
            STATUS_INDICATOR_SET_R3,
            marked(0, LV1_DUPLICATE_ENTRY),
            built(0, 0xffff, 0xfff9),
        ),
    ];
    assert_eq!(cases.len(), 6);
    for (r3, expected, instructions) in cases {
        assert_eq!(r3, expected, "0x{expected:016X}");
        assert_eq!(r3, instructions, "0x{instructions:016X}");
    }
}

/// - [OFW-4.93 lv1 a:0x30829C-0x3082A0] `cmpldi cr7, r31, 4` refuses a
///   bus above 4 over all 64 bits.
/// - [OFW-4.93 lv1 a:0x3082A4-0x3082A8] `cmpdi cr7, r31, 1` refuses each
///   other bus.
/// - [OFW-4.93 lv1 a:0x3082AC-0x3082B0] `cmpdi cr7, r30, 0` refuses each
///   other device.
#[test]
fn the_device_checks_pass_only_device_0_of_bus_1() {
    let cases = [
        (1, 0, None),
        (0, 0, Some(STATUS_INDICATOR_NO_BUS_R3)),
        (2, 0, Some(STATUS_INDICATOR_NO_BUS_R3)),
        (4, 0, Some(STATUS_INDICATOR_NO_BUS_R3)),
        (5, 0, Some(STATUS_INDICATOR_BUS_RANGE_R3)),
        (0x1_0000_0001, 0, Some(STATUS_INDICATOR_BUS_RANGE_R3)),
        (1, 1, Some(STATUS_INDICATOR_NO_DEVICE_R3)),
        (1, 0x1_0000_0000, Some(STATUS_INDICATOR_NO_DEVICE_R3)),
        (5, 1, Some(STATUS_INDICATOR_BUS_RANGE_R3)),
        (2, 1, Some(STATUS_INDICATOR_NO_BUS_R3)),
    ];
    assert_eq!(cases.len(), 10);
    for (bus, device, expected) in cases {
        assert_eq!(
            status_indicator_device_r3(bus, device),
            expected,
            "bus {bus:#x} device {device:#x}"
        );
    }
}
