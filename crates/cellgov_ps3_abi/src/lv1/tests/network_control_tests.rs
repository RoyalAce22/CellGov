use super::*;
use crate::lv1::call::{self, CallSource, NET_CONTROL};
use crate::lv1::status::{LV1_ILLEGAL_PARAMETER_VALUE, LV1_NO_ENTRY};
use crate::lv1::table::{table_entry, TableEntry};

/// The r3 of a check whose high word holds `mark`.
fn marked(mark: u64, status: i64) -> u64 {
    (mark << 32) | zero_extended(status)
}

/// The row is the `LV1_CALL` row of Linux `lv1call.h` for 194.
#[test]
fn net_control_carries_its_header_number_name_and_counts() {
    assert_eq!(NET_CONTROL, 194);
    let row = call::lookup(NET_CONTROL).map(|c| (c.name, c.inputs, c.outputs, c.source));
    assert_eq!(row, Some(("net_control", 6, 2, CallSource::Linux)));
}

#[test]
fn the_4_93_handler_is_the_live_table_entry_of_194() {
    assert_eq!(NET_CONTROL_HANDLERS.len(), 1);
    let handler = net_control_handler("4.93").expect("the 4.93 handler");
    assert_eq!(handler.handler, 0x307D2C);
    assert_eq!(
        table_entry("4.93", NET_CONTROL),
        Some(TableEntry::Handler(handler.handler))
    );
}

#[test]
fn no_net_control_handler_row_exists_for_a_firmware_whose_lv1_is_not_read() {
    for firmware in ["3.70", "4.92", "4.94", ""] {
        assert_eq!(net_control_handler(firmware), None, "{firmware:?}");
    }
}

/// - [OFW-4.93 lv1 a:0x307FB0-0x307FBC] builds -17 with mark 1.
/// - [OFW-4.93 lv1 a:0x307F0C-0x307F18] builds -6 with mark 1.
/// - [OFW-4.93 lv1 a:0x307E6C-0x307E78] builds -6 with mark 2.
/// - [OFW-4.93 lv1 a:0x307F9C-0x307FA8] builds -17 with mark 4.
#[test]
fn each_marked_status_is_its_lv1_status_with_the_mark_in_the_high_word() {
    assert_eq!(
        NET_CONTROL_BUS_RANGE_R3,
        marked(1, LV1_ILLEGAL_PARAMETER_VALUE)
    );
    assert_eq!(NET_CONTROL_NO_BUS_R3, marked(1, LV1_NO_ENTRY));
    assert_eq!(NET_CONTROL_NO_DEVICE_R3, marked(2, LV1_NO_ENTRY));
    assert_eq!(
        NET_CONTROL_REQUEST_RANGE_R3,
        marked(4, LV1_ILLEGAL_PARAMETER_VALUE)
    );
    assert_eq!(NET_CONTROL_WRONG_STATE_R3, 0xFFFF_FFF5);
    assert_eq!(NET_CONTROL_NOT_IMPLEMENTED_R3, 0xFFFF_FFEC);
}

/// [OFW-4.93 lv1 a:0x307DD4-0x307DE8] compares all 64 bits of r3 and r4.
#[test]
fn the_device_checks_run_range_then_bus_then_device() {
    let cases = [
        (5, 1, Some(NET_CONTROL_BUS_RANGE_R3)),
        ((1 << 32) | 1, 0, Some(NET_CONTROL_BUS_RANGE_R3)),
        (u64::MAX, 0, Some(NET_CONTROL_BUS_RANGE_R3)),
        (0, 0, Some(NET_CONTROL_NO_BUS_R3)),
        (4, 1, Some(NET_CONTROL_NO_BUS_R3)),
        (1, 1, Some(NET_CONTROL_NO_DEVICE_R3)),
        (1, 1 << 32, Some(NET_CONTROL_NO_DEVICE_R3)),
        (1, 0, None),
    ];
    assert_eq!(cases.len(), 8);
    for (bus, device, r3) in cases {
        assert_eq!(
            net_control_device_r3(bus, device),
            r3,
            "bus {bus:#x} device {device:#x}"
        );
    }
}

/// [OFW-4.93 lv1 a:0x307F40-0x307F48] passes a code minus 1 up to 10.
/// [OFW-4.93 lv1 a:0x307F84-0x307F98] passes a code minus 1 up to
/// 0x8000000000000001.
#[test]
fn the_request_check_passes_1_to_its_maximum_unsigned() {
    let cases = [
        (
            0,
            Some(NET_CONTROL_REQUEST_RANGE_R3),
            Some(NET_CONTROL_REQUEST_RANGE_R3),
        ),
        (1, None, None),
        (11, None, None),
        (12, Some(NET_CONTROL_REQUEST_RANGE_R3), None),
        (
            0x8000_0000_0000_0002,
            Some(NET_CONTROL_REQUEST_RANGE_R3),
            None,
        ),
        (
            0x8000_0000_0000_0003,
            Some(NET_CONTROL_REQUEST_RANGE_R3),
            Some(NET_CONTROL_REQUEST_RANGE_R3),
        ),
        (
            u64::MAX,
            Some(NET_CONTROL_REQUEST_RANGE_R3),
            Some(NET_CONTROL_REQUEST_RANGE_R3),
        ),
    ];
    assert_eq!(cases.len(), 7);
    for (request, plain, privileged) in cases {
        assert_eq!(
            net_control_request_range_r3(request, false),
            plain,
            "request {request:#x}"
        );
        assert_eq!(
            net_control_request_range_r3(request, true),
            privileged,
            "privileged request {request:#x}"
        );
    }
}
