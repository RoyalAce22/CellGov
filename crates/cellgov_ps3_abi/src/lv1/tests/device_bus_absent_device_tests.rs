use super::*;

/// [OFW-4.93 lv1 a:0x2365E4-0x2365EC] compares the device id unsigned
/// with 6 and branches to the table read when it is not above 6.
#[test]
fn the_storage_bus_lookup_reads_its_table_up_to_device_6() {
    assert_eq!(STORAGE_DEVICE_ID_MAX, 6);
}

/// [OFW-4.93 lv1 a:0x2365F0-0x2365F8] gives 0 for a device id above 6
/// other than -1.
#[test]
fn a_bus_4_device_above_6_other_than_minus_1_is_absent() {
    let devices = [
        7,
        8,
        0xFFFF_FFFF,
        0x1_0000_0000,
        0x8000_0000_0000_0000,
        u64::MAX - 1,
    ];
    assert_eq!(devices.len(), 6);
    for device in devices {
        assert!(storage_bus_device_absent(4, device), "device {device:#x}");
    }
}

#[test]
fn table_devices_the_storage_bus_and_other_buses_are_not_absent() {
    let cases = [
        (4, 0),
        (4, 6),
        (4, u64::MAX),
        (1, 7),
        (2, 7),
        (3, u64::MAX - 1),
        (5, 7),
        (0x0000_0001_0000_0004, 7),
    ];
    assert_eq!(cases.len(), 8);
    for (bus, device) in cases {
        assert!(
            !storage_bus_device_absent(bus, device),
            "bus {bus:#x} device {device:#x}"
        );
    }
}
