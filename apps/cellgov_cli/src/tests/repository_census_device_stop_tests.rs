use super::*;
use cellgov_ps3_abi::lv1::device_bus::{mmio_region, MMIO_REAL_BASE};

/// An access of hardware thread 1 to the first word of the 4.93 region
/// in slot 2 of device 1 of bus 1.
fn census_access(pc: Option<u64>) -> DeviceMmioAccess {
    let region = *mmio_region("4.93", 1, 1, 0x381_0000).expect("the 4.93 region");
    DeviceMmioAccess {
        thread: 1,
        pc,
        region,
        bus_address: region.address,
        real: MMIO_REAL_BASE + region.address,
        ea: 0x8000_0001_0001_1000,
    }
}

#[test]
fn a_device_mmio_stop_names_the_device_its_region_and_each_address_of_the_access() {
    let stop = KernelRunStop::DeviceMmio(census_access(Some(0x8000_0000_0024_590C)));
    assert_eq!(
        stop_text(&stop),
        "an access to the MMIO of bus 1 device 1 region 0x3810000 (slot 2) at bus address \
         0x3810000, real address 0x24003810000 and EA 0x8000000100011000 by hardware thread 1 \
         at pc 0x800000000024590c, which CellGov does not model"
    );
}

#[test]
fn a_device_mmio_stop_with_no_pc_leaves_the_pc_out() {
    let stop = KernelRunStop::DeviceMmio(census_access(None));
    assert_eq!(
        stop_text(&stop),
        "an access to the MMIO of bus 1 device 1 region 0x3810000 (slot 2) at bus address \
         0x3810000, real address 0x24003810000 and EA 0x8000000100011000 by hardware thread 1, \
         which CellGov does not model"
    );
}
