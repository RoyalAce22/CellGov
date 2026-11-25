use super::*;
use crate::hw::device_register::DEVICE_REGISTER_READS;
use crate::lv1::device_bus::mmio_region;
use crate::lv1::repository::{field, first_field, NODES};

/// The first value of the bus node of lpar 1 whose last three key words
/// are `n`, at `firmware`, or `None` when the repository table holds no
/// such node.
fn bus_value(firmware: &str, bus: u64, n: [u64; 3]) -> Option<u64> {
    NODES
        .iter()
        .find(|node| {
            let key = &node.key;
            key.n1 == first_field(b"bus", bus)
                && [key.n2, key.n3, key.n4] == n
                && node.firmware.contains(&firmware)
        })
        .map(|node| node.v1)
}

#[test]
fn each_controller_region_has_the_ehci_region_type_of_a_usb_device() {
    assert_eq!(EHCI_CONTROLLERS.len(), 1);
    for c in EHCI_CONTROLLERS {
        let dev = field(b"dev", c.device);
        let slot = (0..8)
            .find(|&k| {
                bus_value(
                    c.firmware,
                    c.bus,
                    [dev, field(b"reg", k), field(b"data", 0)],
                ) == Some(c.region)
            })
            .unwrap_or_else(|| panic!("no repository region at 0x{:x}", c.region));
        let region_type = [dev, field(b"reg", slot), field(b"type", 0)];
        assert_eq!(
            bus_value(c.firmware, c.bus, region_type),
            Some(EHCI_REGION_TYPE),
            "{c:?}"
        );
        // Linux `arch/powerpc/platforms/ps3/platform.h`: PS3_DEV_TYPE_SB_USB = 4.
        let device_type = [dev, field(b"type", 0), 0];
        assert_eq!(bus_value(c.firmware, c.bus, device_type), Some(4), "{c:?}");
    }
}

#[test]
fn each_controller_operational_base_is_the_captured_capability_length() {
    assert_eq!(EHCI_CONTROLLERS.len(), 1);
    for c in EHCI_CONTROLLERS {
        let region = mmio_region(c.firmware, c.bus, c.device, c.region)
            .unwrap_or_else(|| panic!("no MMIO region for {c:?}"));
        assert!(c.operational + USBINTR + 4 <= region.size, "{c:?}");
        let length = DEVICE_REGISTER_READS
            .iter()
            .find(|r| {
                (r.firmware, r.bus, r.device, r.region, r.offset)
                    == (c.firmware, c.bus, c.device, c.region, 0)
            })
            .unwrap_or_else(|| panic!("no capability read for {c:?}"));
        assert_eq!(length.known, 0xFF, "{c:?}");
        assert_eq!(c.operational, u64::from(length.value), "{c:?}");
    }
}

#[test]
fn a_reader_names_its_store_exactly_when_it_passes_bits_back() {
    assert_eq!(EHCI_RESET_READERS.len(), 4);
    for r in EHCI_RESET_READERS {
        assert_eq!(r.writer.is_some(), r.passes != 0, "{r:?}");
        if let Some(writer) = r.writer {
            assert!(writer > r.reader && writer % 4 == 0, "{r:?}");
        }
    }
}
