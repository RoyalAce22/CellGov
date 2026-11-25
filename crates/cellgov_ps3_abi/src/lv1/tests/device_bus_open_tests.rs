use super::*;
use crate::lv1::call::{self, CallSource, OPEN_DEVICE};
use crate::lv1::status::{zero_extended, LV1_ILLEGAL_PARAMETER_VALUE, LV1_NO_ENTRY};
use crate::lv1::table::{table_entry, TableEntry};

/// The r3 of a check whose high word holds `mark`.
fn marked(mark: u64, status: i64) -> u64 {
    (mark << 32) | zero_extended(status)
}

/// The row is the `LV1_CALL` row of Linux `lv1call.h` for 170.
#[test]
fn open_device_carries_its_header_number_name_and_counts() {
    assert_eq!(OPEN_DEVICE, 170);
    let row = call::lookup(OPEN_DEVICE).map(|c| (c.name, c.inputs, c.outputs, c.source));
    assert_eq!(row, Some(("open_device", 3, 0, CallSource::Linux)));
}

#[test]
fn the_4_93_handler_is_the_live_table_entry_of_170() {
    assert_eq!(OPEN_DEVICE_HANDLERS.len(), 1);
    let handler = open_device_handler("4.93").expect("the 4.93 handler");
    assert_eq!(handler.handler, 0x305A38);
    assert_eq!(
        table_entry("4.93", OPEN_DEVICE),
        Some(TableEntry::Handler(handler.handler))
    );
}

#[test]
fn no_open_device_handler_row_exists_for_a_firmware_whose_lv1_is_not_read() {
    for firmware in ["3.70", "4.92", ""] {
        assert_eq!(open_device_handler(firmware), None, "{firmware:?}");
    }
}

/// [OFW-4.93 lv1 a:0x305B78-0x305B84] `li 1; sldi 32; oris 0xffff; ori 0xffef`;
/// [OFW-4.93 lv1 a:0x237724-0x237750] `li 1; sldi 32; oris 0xffff; ori 0xfffa`;
/// [OFW-4.93 lv1 a:0x237780-0x23778C] `li 2; sldi 32; oris 0xffff; ori 0xfffa`.
#[test]
fn each_check_r3_marks_the_argument_it_reads_over_its_status() {
    let cases = [
        (
            OPEN_DEVICE_BUS_RANGE_R3,
            marked(1, LV1_ILLEGAL_PARAMETER_VALUE),
        ),
        (OPEN_DEVICE_NO_BUS_R3, marked(1, LV1_NO_ENTRY)),
        (OPEN_DEVICE_NO_DEVICE_R3, marked(2, LV1_NO_ENTRY)),
    ];
    assert_eq!(cases.len(), 3);
    for (r3, expected) in cases {
        assert_eq!(r3, expected, "0x{expected:016X}");
    }
}

/// [OFW-4.93 lv1 a:0x2280A4] `li r9, -9`; [OFW-4.93 lv1 a:0x2376C4]
/// `clrldi r3, r3, 32`.
#[test]
fn an_open_device_gets_busy_zero_extended() {
    assert_eq!(OPEN_DEVICE_BUSY_R3, 0x0000_0000_FFFF_FFF7);
}

/// [OFW-4.93 lv1 a:0x305AC8-0x305ACC] `cmpldi cr7, r31, 4` refuses above 4
/// and compares all 64 bits, so a high word over a low word of 1 is above 4;
/// [OFW-4.93 lv1 a:0x2375C0-0x2375E8] [OFW-4.93 lv1 a:0x237714-0x237720]
/// passes 1, 2, 3 and 4.
#[test]
fn the_bus_checks_refuse_0_and_each_id_above_4() {
    let cases = [
        (0, Some(OPEN_DEVICE_NO_BUS_R3)),
        (1, None),
        (2, None),
        (3, None),
        (4, None),
        (5, Some(OPEN_DEVICE_BUS_RANGE_R3)),
        (0x0000_0001_0000_0001, Some(OPEN_DEVICE_BUS_RANGE_R3)),
        (u64::MAX, Some(OPEN_DEVICE_BUS_RANGE_R3)),
    ];
    assert_eq!(cases.len(), 8);
    for (bus, expected) in cases {
        assert_eq!(open_device_bus_r3(bus), expected, "bus {bus}");
    }
}

/// The 4.93 kernel's call names bus 1 and device 0
/// [OFW-4.93 lv2_kernel a:0x80000000000936CC-0x80000000000936EC].
#[test]
fn the_read_open_path_is_device_0_of_bus_1() {
    assert_eq!((OPEN_DEVICE_READ_BUS, OPEN_DEVICE_READ_DEVICE), (1, 0));
    assert_eq!(open_device_bus_r3(OPEN_DEVICE_READ_BUS), None);
}
