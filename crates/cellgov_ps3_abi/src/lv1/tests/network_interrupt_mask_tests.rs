use super::*;
use crate::lv1::call::{self, CallSource, NET_SET_INTERRUPT_MASK};
use crate::lv1::status::{LV1_ILLEGAL_PARAMETER_VALUE, LV1_NO_ENTRY};
use crate::lv1::table::{table_entry, TableEntry};

/// The r3 of a check whose high word holds `mark`.
fn marked(mark: u64, status: i64) -> u64 {
    (mark << 32) | zero_extended(status)
}

/// The row is the `LV1_CALL` row of Linux `lv1call.h` for 193.
#[test]
fn net_set_interrupt_mask_carries_its_header_number_name_and_counts() {
    assert_eq!(NET_SET_INTERRUPT_MASK, 193);
    let row = call::lookup(NET_SET_INTERRUPT_MASK).map(|c| (c.name, c.inputs, c.outputs, c.source));
    assert_eq!(
        row,
        Some(("net_set_interrupt_mask", 4, 0, CallSource::Linux))
    );
}

#[test]
fn the_4_93_handler_is_the_live_table_entry_of_193() {
    assert_eq!(INTERRUPT_MASK_HANDLERS.len(), 1);
    let handler = interrupt_mask_handler("4.93").expect("the 4.93 handler");
    assert_eq!(handler.handler, 0x307FE0);
    assert_eq!(
        table_entry("4.93", NET_SET_INTERRUPT_MASK),
        Some(TableEntry::Handler(handler.handler))
    );
}

#[test]
fn no_interrupt_mask_handler_row_exists_for_a_firmware_whose_lv1_is_not_read() {
    for firmware in ["3.70", "4.92", "4.94", ""] {
        assert_eq!(interrupt_mask_handler(firmware), None, "{firmware:?}");
    }
}

/// - [OFW-4.93 lv1 a:0x3081D4-0x3081E0] builds -17 with mark 1.
/// - [OFW-4.93 lv1 a:0x3081B4-0x3081C0] builds -6 with mark 1.
/// - [OFW-4.93 lv1 a:0x308108-0x308114] builds -6 with mark 2.
/// - [OFW-4.93 lv1 a:0x3081A0-0x3081AC] builds -17 with mark 8.
/// - [OFW-4.93 lv1 a:0x3080D4-0x3080DC] builds -11 with no mark.
#[test]
fn each_interrupt_mask_status_is_its_lv1_status_with_the_mark_in_the_high_word() {
    assert_eq!(
        INTERRUPT_MASK_BUS_RANGE_R3,
        marked(1, LV1_ILLEGAL_PARAMETER_VALUE)
    );
    assert_eq!(INTERRUPT_MASK_NO_BUS_R3, marked(1, LV1_NO_ENTRY));
    assert_eq!(INTERRUPT_MASK_NO_DEVICE_R3, marked(2, LV1_NO_ENTRY));
    assert_eq!(INTERRUPT_MASK_R6_R3, marked(8, LV1_ILLEGAL_PARAMETER_VALUE));
    assert_eq!(INTERRUPT_MASK_WRONG_STATE_R3, 0xFFFF_FFF5);
}

/// [OFW-4.93 lv1 a:0x308070-0x308084] compares all 64 bits of r3 and r4.
#[test]
fn the_interrupt_mask_device_checks_run_range_then_bus_then_device() {
    let cases = [
        (5, 1, Some(INTERRUPT_MASK_BUS_RANGE_R3)),
        ((1 << 32) | 1, 0, Some(INTERRUPT_MASK_BUS_RANGE_R3)),
        (u64::MAX, 0, Some(INTERRUPT_MASK_BUS_RANGE_R3)),
        (0, 0, Some(INTERRUPT_MASK_NO_BUS_R3)),
        (4, 1, Some(INTERRUPT_MASK_NO_BUS_R3)),
        (1, 1, Some(INTERRUPT_MASK_NO_DEVICE_R3)),
        (1, 1 << 32, Some(INTERRUPT_MASK_NO_DEVICE_R3)),
        (1, 0, None),
    ];
    assert_eq!(cases.len(), 8);
    for (bus, device, r3) in cases {
        assert_eq!(
            interrupt_mask_device_r3(bus, device),
            r3,
            "bus {bus:#x} device {device:#x}"
        );
    }
}
