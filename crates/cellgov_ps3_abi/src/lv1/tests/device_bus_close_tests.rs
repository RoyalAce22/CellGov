use super::*;
use crate::lv1::call::CLOSE_DEVICE;
use crate::lv1::table::LIVE_TABLE_HANDLERS;

#[test]
fn the_4_93_close_device_handler_is_the_live_table_entry_of_171() {
    let live = LIVE_TABLE_HANDLERS
        .iter()
        .find(|(number, _)| u64::from(*number) == CLOSE_DEVICE)
        .map(|(_, handler)| *handler);
    assert_eq!(live, Some(0x306EEC));
    assert_eq!(
        close_device_handler("4.93").map(|handler| handler.handler),
        live
    );
}

#[test]
fn no_close_device_handler_row_exists_for_a_firmware_whose_lv1_is_not_read() {
    assert_eq!(CLOSE_DEVICE_HANDLERS.len(), 1);
    for firmware in ["4.92", "4.91", "3.55", "1.00", ""] {
        assert_eq!(close_device_handler(firmware), None, "{firmware}");
    }
}

/// [OFW-4.93 lv1 a:0x306F74-0x306F78] compares the bus id unsigned with
/// 4; [OFW-4.93 lv1 a:0x30701C-0x30702C] builds 1 << 32 | 0xFFFFFFEF.
#[test]
fn a_bus_above_4_gets_minus_17_with_r3_marked() {
    let buses = [5, 6, 0xFFFF_FFFF, 0x1_0000_0001, u64::MAX];
    assert_eq!(buses.len(), 5);
    for bus in buses {
        assert_eq!(
            close_device_check_r3(bus, 0),
            Some(0x0000_0001_FFFF_FFEF),
            "bus {bus:#x}"
        );
    }
}

/// [OFW-4.93 lv1 a:0x236420-0x236428] gives -6 with mark 1;
/// [OFW-4.93 lv1 a:0x237530-0x23753C] sets bit 32.
#[test]
fn bus_0_gets_minus_6_with_r3_marked_for_any_device() {
    for device in [0, 1, 7, u64::MAX] {
        assert_eq!(
            close_device_check_r3(0, device),
            Some(0x0000_0001_FFFF_FFFA),
            "device {device:#x}"
        );
    }
}

/// [OFW-4.93 lv1 a:0x2364EC-0x2364F4] sends a device id above 6 to the
/// -1 test; [OFW-4.93 lv1 a:0x236554-0x236558] and
/// [OFW-4.93 lv1 a:0x236464-0x236474] give -6 with mark 2, so bit 33.
#[test]
fn a_bus_4_device_above_6_other_than_minus_1_gets_minus_6_with_r4_marked() {
    let devices = [7, 0xFFFF_FFFF, 0x8000_0000_0000_0000, u64::MAX - 1];
    assert_eq!(devices.len(), 4);
    for device in devices {
        assert_eq!(
            close_device_check_r3(4, device),
            Some(0x0000_0002_FFFF_FFFA),
            "device {device:#x}"
        );
    }
}

#[test]
fn devices_that_the_access_check_looks_up_pass_the_checks_that_read_no_state() {
    let cases = [
        (1, 0),
        (1, 1),
        (1, 2),
        (1, 7),
        (2, 0),
        (3, 9),
        (4, 0),
        (4, 6),
        (4, u64::MAX),
    ];
    assert_eq!(cases.len(), 9);
    for (bus, device) in cases {
        assert_eq!(
            close_device_check_r3(bus, device),
            None,
            "bus {bus:#x} device {device:#x}"
        );
    }
}

/// [OFW-4.93 lv1 a:0x236538] gives -11 with mark 0, so
/// [OFW-4.93 lv1 a:0x237524-0x23752C] returns it with no high bit.
#[test]
fn a_device_that_the_caller_does_not_hold_open_gets_minus_11_zero_extended() {
    assert_eq!(CLOSE_DEVICE_NOT_OPEN_R3, 0x0000_0000_FFFF_FFF5);
}
