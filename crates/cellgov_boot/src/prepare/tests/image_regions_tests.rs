//! The guest address space marks RSX local memory caching-inhibited.

use cellgov_mem::RegionAccess;

use super::image_regions;

#[test]
fn only_the_rsx_region_is_caching_inhibited_in_every_access_mode() {
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
        assert_eq!(inhibited, ["rsx"], "{rsx_access:?}");
        let rsx = regions.iter().find(|r| r.label() == "rsx").expect("rsx");
        assert_eq!(rsx.access(), rsx_access);
    }
}
