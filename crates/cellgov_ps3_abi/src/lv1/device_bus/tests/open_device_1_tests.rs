use super::*;

/// The 4.93 kernel opens devices 1 and 2 of bus 1
/// [OFW-4.93 lv2_kernel a:0x8000000000093634-0x8000000000093664]
/// [OFW-4.93 lv2_kernel a:0x8000000000093484-0x8000000000093490], and
/// lv1's open function takes a path of its own for each
/// [OFW-4.93 lv1 a:0x237758-0x237764] [OFW-4.93 lv1 a:0x2377DC-0x2377F0].
#[test]
fn the_read_open_paths_are_devices_0_to_2_of_bus_1() {
    assert_eq!(OPEN_DEVICE_READ_DEVICES, [0, 1, 2]);
    let cases = [
        (1, 0, true),
        (1, 1, true),
        (1, 2, true),
        (1, 3, false),
        (1, 0x0000_0001_0000_0001, false),
        (2, 1, false),
        (4, 1, false),
        (0, 1, false),
    ];
    assert_eq!(cases.len(), 8);
    for (bus, device, read) in cases {
        assert_eq!(
            open_device_path_read(bus, device),
            read,
            "bus {bus} device {device}"
        );
    }
}
