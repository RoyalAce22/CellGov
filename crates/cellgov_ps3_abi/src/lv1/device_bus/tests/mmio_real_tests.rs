//! The real address of an MMIO region of a system bus device.

use super::*;

#[test]
fn each_4_93_mmio_region_lies_at_the_real_base_plus_its_bus_address() {
    let reals: Vec<_> = MMIO_REGIONS
        .iter()
        .map(|r| mmio_real_address(r.firmware, r.bus, r.device, r.address))
        .collect();
    assert_eq!(
        reals,
        [
            Some(0x240_0000_2400),
            Some(0x240_0301_0000),
            Some(0x240_0381_0000),
            Some(0x240_0000_2600),
            Some(0x240_0302_0000),
            Some(0x240_0382_0000),
        ]
    );
}

#[test]
fn a_bus_address_that_no_region_has_gets_no_real_address() {
    assert_eq!(mmio_real_address("4.93", 1, 1, 0x302_0000), None);
    assert_eq!(mmio_real_address("4.92", 1, 1, 0x301_0000), None);
    assert_eq!(mmio_real_address("4.93", 1, 2, 0x301_0000), None);
}
