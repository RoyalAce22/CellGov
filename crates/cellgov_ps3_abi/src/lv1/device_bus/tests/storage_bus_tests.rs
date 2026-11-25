use super::*;

#[test]
fn the_storage_bus_is_device_minus_1_of_bus_4() {
    assert_eq!(STORAGE_BUS, 4);
    assert_eq!(STORAGE_BUS_DEVICE, -1i64 as u64);
    assert!(opens_storage_bus(4, u64::MAX));
}

#[test]
fn the_handler_bus_checks_pass_the_storage_bus() {
    assert_eq!(open_device_bus_r3(STORAGE_BUS), None);
}

#[test]
fn no_other_device_or_bus_opens_the_storage_bus() {
    let cases = [
        (4, 0),
        (4, 6),
        (4, 7),
        (4, u64::MAX - 1),
        (4, 0x0000_0000_FFFF_FFFF),
        (1, u64::MAX),
        (3, u64::MAX),
        (5, u64::MAX),
        (0x0000_0001_0000_0004, u64::MAX),
    ];
    assert_eq!(cases.len(), 9);
    for (bus, device) in cases {
        assert!(
            !opens_storage_bus(bus, device),
            "bus {bus:#x} device {device:#x}"
        );
    }
}

#[test]
fn an_open_of_the_held_storage_bus_returns_busy_zero_extended() {
    assert_eq!(OPEN_STORAGE_BUS_BUSY_R3, 0x0000_0000_FFFF_FFF7);
}

#[test]
fn the_storage_bus_open_capture_has_its_readme() {
    let readme = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(STORAGE_BUS_OPEN_CAPTURE)
        .join("README.md");
    assert!(readme.is_file(), "{}", readme.display());
}
