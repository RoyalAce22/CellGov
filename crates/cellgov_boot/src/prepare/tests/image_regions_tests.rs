//! The guest address space marks RSX local memory and the raw SPU windows
//! caching-inhibited.

use cellgov_mem::{GuestMemory, RegionAccess};
use cellgov_ps3_abi::lv2::spu::raw_window;

use super::image_regions;

#[test]
fn only_the_rsx_and_spu_regions_are_caching_inhibited_in_every_access_mode() {
    for rsx_access in [
        RegionAccess::ReadWrite,
        RegionAccess::ReservedZeroReadable,
        RegionAccess::ReservedStrict,
    ] {
        let regions = image_regions(0x1_0000, rsx_access, RegionAccess::ReservedZeroReadable);
        let inhibited: Vec<&str> = regions
            .iter()
            .filter(|r| r.is_caching_inhibited())
            .map(|r| r.label())
            .collect();
        assert_eq!(inhibited, ["rsx", "spu_reserved"], "{rsx_access:?}");
        let rsx = regions.iter().find(|r| r.label() == "rsx").expect("rsx");
        assert_eq!(rsx.access(), rsx_access);
    }
}

#[test]
fn a_raw_spu_window_carved_from_the_guest_address_space_is_caching_inhibited() {
    let mut mem = GuestMemory::from_regions(image_regions(
        0x1_0000,
        RegionAccess::ReservedZeroReadable,
        RegionAccess::ReservedZeroReadable,
    ))
    .expect("the boot regions tile");
    let base = raw_window::BASE + raw_window::STRIDE;
    mem.map_window(base, raw_window::STRIDE as usize, "raw_spu_window")
        .expect("the window lies in the reserved SPU range");
    let window = mem
        .regions()
        .find(|r| r.base() == base)
        .expect("the window is a region");
    assert_eq!(window.access(), RegionAccess::ReadWrite);
    assert!(window.is_caching_inhibited(), "spu_raw_window_span");
    let view = mem
        .region_views()
        .find(|v| v.base == base)
        .expect("the window has a view");
    assert!(view.caching_inhibited, "the PPU sees the attribute");
}
