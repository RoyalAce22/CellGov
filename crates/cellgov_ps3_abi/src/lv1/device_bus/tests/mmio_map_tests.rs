use super::*;
use crate::lv1::call::{self, CallSource, MAP_DEVICE_MMIO_REGION};
use crate::lv1::status::{zero_extended, LV1_ILLEGAL_PARAMETER_VALUE, LV1_NO_ENTRY};
use crate::lv1::table::{table_entry, TableEntry};

/// The r3 of a check whose high word holds `bits`.
fn marked(bits: u64, status: i64) -> u64 {
    (bits << 32) | zero_extended(status)
}

/// The row is the `LV1_CALL` row of Linux `lv1call.h` for 172.
#[test]
fn map_device_mmio_region_carries_its_header_number_name_and_counts() {
    assert_eq!(MAP_DEVICE_MMIO_REGION, 172);
    let row = call::lookup(MAP_DEVICE_MMIO_REGION).map(|c| (c.name, c.inputs, c.outputs, c.source));
    assert_eq!(
        row,
        Some(("map_device_mmio_region", 5, 1, CallSource::Linux))
    );
}

#[test]
fn the_4_93_handler_is_the_live_table_entry_of_172() {
    assert_eq!(MMIO_MAP_HANDLERS.len(), 1);
    let handler = mmio_map_handler("4.93").expect("the 4.93 handler");
    assert_eq!(handler.handler, 0x306D64);
    assert_eq!(
        table_entry("4.93", MAP_DEVICE_MMIO_REGION),
        Some(TableEntry::Handler(handler.handler))
    );
    for firmware in ["3.70", "4.92", ""] {
        assert_eq!(mmio_map_handler(firmware), None, "{firmware:?}");
    }
}

/// [OFW-4.93 lv1 a:0x306ED8-0x306EE4] `li 1; sldi 32; oris 0xffff; ori 0xffef`;
/// [OFW-4.93 lv1 a:0x236420-0x236428] mark 1 and -6;
/// [OFW-4.93 lv1 a:0x236538] -11 with mark 0;
/// [OFW-4.93 lv1 a:0x23739C-0x2373A8] `li 4; sldi 32; oris 0xffff; ori 0xffef`;
/// [OFW-4.93 lv1 a:0x228CA4-0x228CAC] mark 3 and -17;
/// [OFW-4.93 lv1 a:0x228DF4-0x228DFC] mark 4 and -17;
/// [OFW-4.93 lv1 a:0x228DE4-0x228DEC] mark 5 and -17.
/// [OFW-4.93 lv1 a:0x237340-0x237390] sets bit (mark + 31).
#[test]
fn each_mmio_map_check_r3_puts_its_mark_over_its_status() {
    let cases = [
        (
            MMIO_MAP_BUS_RANGE_R3,
            marked(1, LV1_ILLEGAL_PARAMETER_VALUE),
        ),
        (MMIO_MAP_NO_BUS_R3, marked(1, LV1_NO_ENTRY)),
        (MMIO_MAP_NOT_OPEN_R3, 0x0000_0000_FFFF_FFF5),
        (MMIO_MAP_DEVICE_0_R3, marked(4, LV1_ILLEGAL_PARAMETER_VALUE)),
        (
            MMIO_MAP_NO_REGION_R3,
            marked(4, LV1_ILLEGAL_PARAMETER_VALUE),
        ),
        (MMIO_MAP_SIZE_R3, marked(8, LV1_ILLEGAL_PARAMETER_VALUE)),
        (
            MMIO_MAP_PAGE_SHIFT_R3,
            marked(16, LV1_ILLEGAL_PARAMETER_VALUE),
        ),
    ];
    assert_eq!(cases.len(), 7);
    for (r3, expected) in cases {
        assert_eq!(r3, expected, "{r3:#x}");
    }
}

/// [OFW-4.93 lv1 a:0x228D18-0x228D34] `cmpdi r25, 12 / 16 / 20 / 24`.
#[test]
fn the_page_shifts_are_the_four_that_the_map_function_compares() {
    assert_eq!(MMIO_MAP_PAGE_SHIFTS, [12, 16, 20, 24]);
}

/// [OFW-4.93 lv1 a:0x3A9A60-0x3A9A9C] `li r4, i` with
/// `li r5, 9216; li r6, 512`, `lis r5, 0x301; lis r6, 1` and
/// `lis r5, 0x381; lis r6, 1`.
#[test]
fn device_1_has_the_three_regions_of_lv1s_init_in_slot_order() {
    let regions: Vec<_> = MMIO_REGIONS
        .iter()
        .filter(|r| r.device == 1)
        .map(|r| (r.firmware, r.bus, r.device, r.slot, r.address, r.size))
        .collect();
    assert_eq!(
        regions,
        [
            ("4.93", 1, 1, 0, 0x2400, 0x200),
            ("4.93", 1, 1, 1, 0x301_0000, 0x1_0000),
            ("4.93", 1, 1, 2, 0x381_0000, 0x1_0000),
        ]
    );
}

#[test]
fn a_region_matches_only_its_own_firmware_bus_device_and_bus_address() {
    let region = mmio_region("4.93", 1, 1, 0x301_0000).map(|r| r.slot);
    assert_eq!(region, Some(1));
    for (bus, device, address) in [
        (1, 1, 0x301_0001),
        (1, 1, 0x301_8000),
        (1, 1, 0x380_0000),
        (1, 2, 0x301_0000),
        (1, 0, 0x2400),
        (2, 1, 0x2400),
    ] {
        assert_eq!(
            mmio_region("4.93", bus, device, address),
            None,
            "{bus} {device} {address:#x}"
        );
    }
    assert_eq!(mmio_region("4.92", 1, 1, 0x301_0000), None);
}

#[test]
fn the_regions_are_read_only_for_devices_1_and_2_of_bus_1_at_4_93() {
    assert!(mmio_regions_read("4.93", 1, 1));
    assert!(mmio_regions_read("4.93", 1, 2));
    assert!(!mmio_regions_read("4.93", 1, 3));
    assert!(!mmio_regions_read("4.93", 2, 1));
    assert!(!mmio_regions_read("4.92", 1, 1));
}
