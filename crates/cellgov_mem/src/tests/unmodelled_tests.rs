//! Unmodelled spans: which bytes a span holds, what each access there
//! gives, and which declarations the memory refuses.

use super::*;
use crate::{ByteRange, GuestAddr, PageSize, Region};

fn range(addr: u64, len: u64) -> ByteRange {
    ByteRange::new(GuestAddr::new(addr), len).unwrap()
}

/// A 16 KB region at 0 and a 64 KB span at 0x24003810000.
const SPAN: (u64, u64) = (0x240_0381_0000, 0x1_0000);

fn memory() -> GuestMemory {
    let mut mem =
        GuestMemory::from_regions(vec![Region::new(0, 0x4000, "main", PageSize::Page4K)]).unwrap();
    mem.declare_unmodelled(SPAN.0, SPAN.1).unwrap();
    mem
}

#[test]
fn a_span_holds_its_first_and_last_byte_and_no_byte_outside() {
    let mem = memory();
    let (base, len) = SPAN;
    assert!(mem.is_unmodelled(base));
    assert!(mem.is_unmodelled(base + len - 1));
    assert!(!mem.is_unmodelled(base - 1));
    assert!(!mem.is_unmodelled(base + len));
    assert!(!mem.is_unmodelled(0));
    assert_eq!(mem.unmodelled_spans(), [(base, base + len)]);
}

#[test]
fn a_span_reads_no_bytes_and_refuses_a_commit() {
    let mut mem = memory();
    let at = range(SPAN.0, 4);
    assert_eq!(mem.read(at), None);
    assert!(matches!(
        mem.read_checked(at),
        Err(MemError::Unmapped(FaultContext { addr, .. })) if addr == SPAN.0
    ));
    assert!(!mem.holds(SPAN.0, 4));
    assert!(matches!(
        mem.apply_commit(at, &[1; 4]),
        Err(MemError::Unmapped(_))
    ));
    assert!(mem.region_views().all(|v| v.base != SPAN.0));
}

#[test]
fn a_span_over_a_region_or_a_span_is_refused_and_a_region_over_a_span_too() {
    let mut mem = memory();
    assert_eq!(
        mem.declare_unmodelled(0x3FFF, 2),
        Err(MemError::OverlappingRegions)
    );
    assert_eq!(
        mem.declare_unmodelled(SPAN.0 + SPAN.1 - 1, 0x10),
        Err(MemError::OverlappingRegions)
    );
    assert_eq!(
        mem.install_region(SPAN.0 - 0x1000, 0x1001, "over", PageSize::Page4K),
        Err(MemError::OverlappingRegions)
    );
    assert_eq!(mem.unmodelled_spans(), [(SPAN.0, SPAN.0 + SPAN.1)]);
    assert_eq!(mem.regions().count(), 1);
}

#[test]
fn spans_beside_a_span_or_a_region_hold_their_bytes_in_address_order() {
    let mut mem = memory();
    let (base, len) = SPAN;
    let below = base - 0x200;
    let above = base + len;
    mem.declare_unmodelled(above, 0x200).unwrap();
    mem.declare_unmodelled(below, 0x200).unwrap();
    mem.declare_unmodelled(0x4000, 0x10).unwrap();
    assert_eq!(
        mem.unmodelled_spans(),
        [
            (0x4000, 0x4010),
            (below, base),
            (base, above),
            (above, above + 0x200)
        ]
    );
    assert!(mem.is_unmodelled(0x4000));
    assert!(!mem.is_unmodelled(0x4010));
    assert!(mem.is_unmodelled(below));
    assert!(mem.is_unmodelled(base - 1));
    assert!(mem.is_unmodelled(above));
}

#[test]
fn a_span_of_no_bytes_records_nothing() {
    let mut mem = memory();
    mem.declare_unmodelled(0x4000, 0).unwrap();
    assert_eq!(mem.unmodelled_spans(), [(SPAN.0, SPAN.0 + SPAN.1)]);
    assert!(!mem.is_unmodelled(0x4000));
}

#[test]
fn a_region_beside_a_span_on_either_side_is_installed() {
    let mut mem = memory();
    let (base, len) = SPAN;
    mem.install_region(base - 0x1000, 0x1000, "below", PageSize::Page4K)
        .unwrap();
    mem.install_region(base + len, 0x1000, "above", PageSize::Page4K)
        .unwrap();
    let bases: Vec<u64> = mem.regions().map(Region::base).collect();
    assert_eq!(bases, [0, base - 0x1000, base + len]);
    assert_eq!(mem.unmodelled_spans(), [(base, base + len)]);
}

#[test]
fn a_span_past_the_end_of_the_address_space_is_refused() {
    let mut mem = memory();
    assert_eq!(
        mem.declare_unmodelled(u64::MAX, 2),
        Err(MemError::RegionOverflow {
            base: u64::MAX,
            size: 2
        })
    );
    assert_eq!(mem.unmodelled_spans().len(), 1);
}

#[test]
fn a_span_leaves_the_content_hash_as_it_is() {
    let plain =
        GuestMemory::from_regions(vec![Region::new(0, 0x4000, "main", PageSize::Page4K)]).unwrap();
    assert_eq!(memory().content_hash(), plain.content_hash());
}
