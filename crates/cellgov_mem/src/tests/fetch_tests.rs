//! The unmapped floor and the code spans: what each access below the
//! floor gives, how declared spans merge, and which span a fetch reads.

use super::*;
use crate::{ByteRange, GuestAddr, PageSize, Region};

fn range(addr: u64, len: u64) -> ByteRange {
    ByteRange::new(GuestAddr::new(addr), len).unwrap()
}

fn memory() -> GuestMemory {
    GuestMemory::from_regions(vec![
        Region::new(0, 0x4000, "main", PageSize::Page4K),
        Region::new(0x8000, 0x1000, "stack", PageSize::Page4K),
    ])
    .unwrap()
}

#[test]
fn every_access_below_the_floor_is_refused_and_the_floor_itself_is_not() {
    let mut mem = memory();
    mem.unmap_below(0x1000);
    assert!(matches!(
        mem.read_checked(range(0xFFC, 4)),
        Err(MemError::Unmapped(FaultContext { addr: 0xFFC, .. }))
    ));
    assert!(matches!(
        mem.apply_commit(range(0, 4), &[1; 4]),
        Err(MemError::Unmapped(_))
    ));
    assert_eq!(mem.read(range(0x1000, 4)).as_deref(), Some(&[0u8; 4][..]));
    assert_eq!(mem.apply_commit(range(0x1000, 4), &[1; 4]), Ok(()));
}

#[test]
fn the_region_view_at_zero_starts_at_the_floor() {
    let mut mem = memory();
    mem.unmap_below(0x1000);
    let views: Vec<(u64, usize)> = mem
        .region_views()
        .map(|v| (v.base, v.bytes.len()))
        .collect();
    assert_eq!(views, [(0x1000, 0x3000), (0x8000, 0x1000)]);
}

#[test]
fn a_floor_past_a_region_drops_its_view() {
    let mut mem = memory();
    mem.unmap_below(0x4000);
    let bases: Vec<u64> = mem.region_views().map(|v| v.base).collect();
    assert_eq!(bases, [0x8000]);
}

#[test]
fn declared_spans_that_overlap_or_touch_merge() {
    let mut mem = memory();
    mem.declare_code(0x2000, 0x100);
    mem.declare_code(0x1000, 0x100);
    mem.declare_code(0x1100, 0x10);
    mem.declare_code(0x1F00, 0x180);
    mem.declare_code(0x3000, 0);
    assert_eq!(mem.code_spans(), [(0x1000, 0x1110), (0x1F00, 0x2100)]);
    mem.declare_code(0x1100, 0xE00);
    assert_eq!(mem.code_spans(), [(0x1000, 0x2100)]);
}

#[test]
fn an_unrestricted_memory_fetches_the_region_at_zero_above_the_floor() {
    let mut mem = memory();
    assert_eq!(mem.fetch_span(0), Ok((0, 0x4000)));
    mem.unmap_below(0x1000);
    assert_eq!(mem.fetch_span(0xFFC), Err(FetchRefusal::Unmapped));
    assert_eq!(mem.fetch_span(0x2000), Ok((0x1000, 0x4000)));
    assert_eq!(mem.fetch_span(0x5000), Err(FetchRefusal::Unmapped));
}

#[test]
fn a_restricted_memory_fetches_only_its_code_spans() {
    let mut mem = memory();
    mem.declare_code(0x1000, 0x100);
    mem.restrict_fetch();
    assert_eq!(mem.fetch_span(0x10FC), Ok((0x1000, 0x1100)));
    assert_eq!(mem.fetch_span(0x1100), Err(FetchRefusal::NoExecute));
    assert_eq!(mem.fetch_span(0xFFC), Err(FetchRefusal::NoExecute));
    assert_eq!(mem.fetch_span(0x8000), Err(FetchRefusal::NoExecute));
    assert_eq!(mem.fetch_span(0x5000), Err(FetchRefusal::Unmapped));
}

#[test]
fn a_code_span_below_the_floor_is_still_unmapped() {
    let mut mem = memory();
    mem.declare_code(0, 0x2000);
    mem.unmap_below(0x1000);
    mem.restrict_fetch();
    assert_eq!(mem.fetch_span(0x800), Err(FetchRefusal::Unmapped));
    assert_eq!(mem.fetch_span(0x1000), Ok((0x1000, 0x2000)));
}
