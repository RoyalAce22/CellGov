//! The install check: it gives the answer that the install gives for each
//! range, and it changes nothing.

use super::*;
use crate::{PageSize, Region};

const REGION: (u64, usize) = (0x1000, 0x100);
const SPAN: (u64, u64) = (0x8000, 0x1000);

fn memory() -> GuestMemory {
    let mut mem = GuestMemory::from_regions(vec![Region::new(
        REGION.0,
        REGION.1,
        "held",
        PageSize::Page4K,
    )])
    .unwrap();
    mem.declare_unmodelled(SPAN.0, SPAN.1).unwrap();
    mem
}

/// Each range that the install refuses, with the reason.
fn refused() -> [((u64, usize), MemError); 6] {
    let (region_base, region_size) = REGION;
    let span_end = SPAN.0 + SPAN.1;
    [
        (
            (u64::MAX - 0x10, 0x40),
            MemError::RegionOverflow {
                base: u64::MAX - 0x10,
                size: 0x40,
            },
        ),
        ((region_base - 0x10, 0x20), MemError::OverlappingRegions),
        (
            (region_base + region_size as u64 - 1, 0x10),
            MemError::OverlappingRegions,
        ),
        ((region_base, 0), MemError::OverlappingRegions),
        ((SPAN.0 - 0x10, 0x20), MemError::OverlappingRegions),
        ((span_end - 1, 0x10), MemError::OverlappingRegions),
    ]
}

/// Each range that the install accepts.
fn accepted() -> [(u64, usize); 4] {
    let region_end = REGION.0 + REGION.1 as u64;
    [
        (REGION.0 - 0x40, 0x40),
        (region_end, 0x40),
        (SPAN.0 - 0x40, 0x40),
        (SPAN.0 + SPAN.1, 0x40),
    ]
}

#[test]
fn the_check_and_the_install_refuse_each_range_for_the_same_reason() {
    let cases = refused();
    assert_eq!(cases.len(), 6);
    for ((base, size), reason) in cases {
        let mut mem = memory();
        let hash = mem.content_hash();
        assert_eq!(
            mem.check_install_region(base, size, &[]),
            Err(reason),
            "the check of 0x{base:x}+0x{size:x}",
        );
        assert_eq!(
            mem.install_region(base, size, "new", PageSize::Page4K),
            Err(reason),
            "the install of 0x{base:x}+0x{size:x}",
        );
        assert_eq!(mem.regions().count(), 1);
        assert_eq!(mem.content_hash(), hash);
    }
}

#[test]
fn the_check_accepts_each_range_that_the_install_accepts_and_installs_nothing() {
    let cases = accepted();
    assert_eq!(cases.len(), 4);
    for (base, size) in cases {
        let mut mem = memory();
        assert_eq!(mem.check_install_region(base, size, &[]), Ok(()));
        assert_eq!(mem.regions().count(), 1, "the check installs nothing");
        assert_eq!(
            mem.install_region(base, size, "new", PageSize::Page4K),
            Ok(())
        );
        assert_eq!(mem.regions().count(), 2);
    }
}

#[test]
fn a_pending_range_refuses_an_overlapping_range_as_an_installed_region_does() {
    let pending = (0x2000, 0x40);
    let mut mem = memory();
    assert_eq!(
        mem.check_install_region(0x2020, 0x40, &[pending]),
        Err(MemError::OverlappingRegions),
    );
    assert_eq!(
        mem.check_install_region(0x1FF0, 0x20, &[pending]),
        Err(MemError::OverlappingRegions),
    );
    assert_eq!(
        mem.check_install_region(0x2000, 0, &[pending]),
        Err(MemError::OverlappingRegions),
    );
    mem.install_region(pending.0, pending.1, "pending", PageSize::Page4K)
        .unwrap();
    for (base, size) in [(0x2020, 0x40), (0x1FF0, 0x20), (0x2000, 0)] {
        assert_eq!(
            mem.install_region(base, size, "new", PageSize::Page4K),
            Err(MemError::OverlappingRegions),
        );
    }
}

#[test]
fn a_range_beside_a_pending_range_is_accepted() {
    let mem = memory();
    let pending = [(0x2000, 0x40)];
    assert_eq!(mem.check_install_region(0x2040, 0x40, &pending), Ok(()));
    assert_eq!(mem.check_install_region(0x1FC0, 0x40, &pending), Ok(()));
}
