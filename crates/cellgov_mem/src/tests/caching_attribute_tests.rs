//! A region's caching attribute reaches its views, survives a window
//! carved from it, and comes back when the window returns.

use crate::{GuestMemory, PageSize, Region, RegionAccess, RegionView};

const RESERVED_BASE: u64 = 0x10_0000;
const WINDOW: u64 = RESERVED_BASE + 0x1_0000;

fn memory() -> GuestMemory {
    GuestMemory::from_regions(vec![
        Region::new(0, 0x1000, "main", PageSize::Page64K),
        Region::with_access(
            RESERVED_BASE,
            0x4_0000,
            "reserved",
            PageSize::Page64K,
            RegionAccess::ReservedZeroReadable,
        )
        .with_caching_inhibited(),
    ])
    .unwrap()
}

/// `(base, size, label, caching_inhibited)` for each view, in order.
fn views(mem: &GuestMemory) -> Vec<(u64, u64, &'static str, bool)> {
    mem.region_views()
        .zip(mem.regions())
        .map(|(v, r)| (v.base, v.bytes.len() as u64, r.label(), v.caching_inhibited))
        .collect()
}

#[test]
fn a_new_region_and_a_plain_view_are_cacheable() {
    assert!(!Region::new(0, 0x1000, "main", PageSize::Page4K).is_caching_inhibited());
    assert!(!RegionView::plain(0, &[0; 4]).caching_inhibited);
}

#[test]
fn each_view_carries_its_region_s_caching_attribute() {
    assert_eq!(
        views(&memory()),
        [
            (0, 0x1000, "main", false),
            (RESERVED_BASE, 0x4_0000, "reserved", true),
        ]
    );
}

#[test]
fn a_window_and_the_pieces_around_it_keep_the_attribute_and_rejoin_with_it() {
    let mut mem = memory();
    mem.map_window(WINDOW, 0x1_0000, "window").unwrap();
    assert_eq!(
        views(&mem)[1..],
        [
            (RESERVED_BASE, 0x1_0000, "reserved", true),
            (WINDOW, 0x1_0000, "window", true),
            (WINDOW + 0x1_0000, 0x2_0000, "reserved", true),
        ]
    );
    mem.unmap_window(WINDOW).unwrap();
    assert_eq!(views(&mem), views(&memory()));
}
