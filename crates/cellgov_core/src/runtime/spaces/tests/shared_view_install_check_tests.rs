//! A shared mapping checks each view with the install's own check before
//! it installs any view, so a refusal installs nothing.

use cellgov_mem::{GuestMemory, MemError, PageSize};
use cellgov_time::Budget;

use super::*;
use crate::runtime::state::Runtime;

const S1: AddressSpaceId = AddressSpaceId::new(1);
const KEY: u64 = 0x31;
const SEG_SIZE: u64 = 0x40;

/// An unmodelled span in boot memory.
const SPAN: (u64, u64) = (0x8000, 0x1000);

/// The base of a view in S1.
const CHILD_VIEW: u64 = 0x3000;

/// A boot region below the span, so boot memory holds a region and a span.
const BOOT_REGION: (u64, usize) = (0x1000, 0x100);

fn runtime() -> Runtime {
    let mut boot = GuestMemory::from_regions(Vec::new()).unwrap();
    boot.install_region(BOOT_REGION.0, BOOT_REGION.1, "boot", PageSize::Page4K)
        .unwrap();
    boot.declare_unmodelled(SPAN.0, SPAN.1).unwrap();
    let mut rt = Runtime::new(boot, Budget::new(4), 100);
    rt.create_address_space(S1).unwrap();
    rt
}

fn region_count(rt: &Runtime, space: AddressSpaceId) -> usize {
    rt.space_memory(space).unwrap().regions().count()
}

#[test]
fn a_later_view_over_an_unmodelled_span_refuses_the_mapping_and_installs_no_view() {
    let mut rt = runtime();
    let hash = rt.committed_memory_hash();
    let err = rt
        .register_shared_mapping(
            KEY,
            SEG_SIZE,
            &[
                (S1, CHILD_VIEW),
                (AddressSpaceId::BOOT, SPAN.0 + SPAN.1 - 0x10),
            ],
        )
        .unwrap_err();
    assert_eq!(err, SpaceError::ViewInstall(MemError::OverlappingRegions));
    assert_eq!(
        region_count(&rt, S1),
        0,
        "the earlier view is not installed"
    );
    assert_eq!(region_count(&rt, AddressSpaceId::BOOT), 1);
    assert_eq!(rt.committed_memory_hash(), hash);
    assert!(!rt.spaces.shared.contains_key(KEY));
}

#[test]
fn a_mapping_that_overlaps_nothing_installs_each_view() {
    let mut rt = runtime();
    rt.register_shared_mapping(
        KEY,
        SEG_SIZE,
        &[(S1, CHILD_VIEW), (AddressSpaceId::BOOT, SPAN.0 + SPAN.1)],
    )
    .unwrap();
    assert_eq!(region_count(&rt, S1), 1);
    assert_eq!(region_count(&rt, AddressSpaceId::BOOT), 2);
    assert!(rt.spaces.shared.contains_key(KEY));
}

/// The boot view of each refused mapping, after a first view in S1, any
/// earlier boot view of the same mapping, and the error of the install.
fn refused_boot_views() -> [(u64, Option<u64>, MemError); 5] {
    let overflow = u64::MAX - 0x10;
    [
        (
            overflow,
            None,
            MemError::RegionOverflow {
                base: overflow,
                size: SEG_SIZE,
            },
        ),
        (
            BOOT_REGION.0 + BOOT_REGION.1 as u64 - 1,
            None,
            MemError::OverlappingRegions,
        ),
        (SPAN.0 - 0x10, None, MemError::OverlappingRegions),
        (SPAN.0 + SPAN.1 - 1, None, MemError::OverlappingRegions),
        (0x5020, Some(0x5000), MemError::OverlappingRegions),
    ]
}

#[test]
fn the_mapping_refuses_each_view_with_the_error_of_the_install() {
    let cases = refused_boot_views();
    assert_eq!(cases.len(), 5);
    for (base, earlier, expected) in cases {
        let mut rt = runtime();
        let hash = rt.committed_memory_hash();
        let mut views = vec![(S1, CHILD_VIEW)];
        views.extend(earlier.map(|e| (AddressSpaceId::BOOT, e)));
        views.push((AddressSpaceId::BOOT, base));

        let mut installed = rt.memory().clone();
        if let Some(e) = earlier {
            installed
                .install_region(e, SEG_SIZE as usize, "shared", PageSize::Page64K)
                .unwrap();
        }
        let reason = installed
            .install_region(base, SEG_SIZE as usize, "shared", PageSize::Page64K)
            .unwrap_err();
        assert_eq!(reason, expected, "the install of 0x{base:x}");

        assert_eq!(
            rt.register_shared_mapping(KEY, SEG_SIZE, &views),
            Err(SpaceError::ViewInstall(reason)),
            "the boot view at 0x{base:x}",
        );
        assert_eq!(region_count(&rt, S1), 0);
        assert_eq!(region_count(&rt, AddressSpaceId::BOOT), 1);
        assert_eq!(rt.committed_memory_hash(), hash);
    }
}
