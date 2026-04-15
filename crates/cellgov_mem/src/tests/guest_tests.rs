use super::*;
use crate::addr::GuestAddr;

fn range(start: u64, length: u64) -> ByteRange {
    ByteRange::new(GuestAddr::new(start), length).unwrap()
}

#[test]
fn new_is_zero_initialized() {
    let mem = GuestMemory::new(16);
    assert_eq!(mem.size(), 16);
    let bytes = mem.read(range(0, 16)).unwrap();
    assert_eq!(bytes, &[0u8; 16]);
}

#[test]
fn read_in_range() {
    let mut mem = GuestMemory::new(16);
    mem.apply_commit(range(4, 4), &[1, 2, 3, 4]).unwrap();
    assert_eq!(mem.read(range(4, 4)).unwrap(), &[1, 2, 3, 4]);
}

#[test]
fn read_zero_length_at_in_bounds_start() {
    let mem = GuestMemory::new(16);
    let s = mem.read(range(8, 0)).unwrap();
    assert!(s.is_empty());
}

#[test]
fn read_zero_length_at_size_boundary() {
    let mem = GuestMemory::new(16);
    let s = mem.read(range(16, 0)).unwrap();
    assert!(s.is_empty());
}

#[test]
fn read_past_end_is_none() {
    let mem = GuestMemory::new(16);
    assert_eq!(mem.read(range(15, 2)), None);
}

#[test]
fn read_starting_past_end_is_none() {
    let mem = GuestMemory::new(16);
    assert_eq!(mem.read(range(17, 1)), None);
}

#[test]
fn commit_writes_visible_on_read() {
    let mut mem = GuestMemory::new(8);
    mem.apply_commit(range(0, 4), &[0xde, 0xad, 0xbe, 0xef])
        .unwrap();
    assert_eq!(mem.read(range(0, 4)).unwrap(), &[0xde, 0xad, 0xbe, 0xef]);
    // Untouched tail still zero.
    assert_eq!(mem.read(range(4, 4)).unwrap(), &[0, 0, 0, 0]);
}

#[test]
fn commit_length_mismatch_rejected() {
    let mut mem = GuestMemory::new(16);
    let err = mem.apply_commit(range(0, 4), &[1, 2, 3]).unwrap_err();
    assert_eq!(err, MemError::LengthMismatch);
    // Memory left untouched.
    assert_eq!(mem.read(range(0, 4)).unwrap(), &[0, 0, 0, 0]);
}

#[test]
fn commit_out_of_range_rejected() {
    let mut mem = GuestMemory::new(8);
    let err = mem.apply_commit(range(6, 4), &[1, 2, 3, 4]).unwrap_err();
    assert!(matches!(err, MemError::Unmapped(_)));
    // Memory left untouched.
    assert_eq!(mem.read(range(0, 8)).unwrap(), &[0; 8]);
}

#[test]
fn read_checked_reports_unmapped_with_nearest_regions() {
    // Two regions with a gap between them: user_heap ends at
    // 0x100, rsx begins at 0x200. An access at 0x150 should name
    // both neighbors.
    let mem = GuestMemory::from_regions(vec![
        Region::new(0, 0x100, "user_heap", PageSize::Page64K),
        Region::new(0x200, 0x100, "rsx", PageSize::Page64K),
    ])
    .unwrap();
    let err = mem.read_checked(range(0x150, 4)).unwrap_err();
    match err {
        MemError::Unmapped(ctx) => {
            assert_eq!(ctx.addr, 0x150);
            assert_eq!(ctx.nearest_below, Some("user_heap"));
            assert_eq!(ctx.nearest_above, Some("rsx"));
        }
        other => panic!("expected Unmapped, got {:?}", other),
    }
}

#[test]
fn fault_context_no_regions_below_returns_none() {
    let mem =
        GuestMemory::from_regions(vec![Region::new(0x1000, 0x100, "heap", PageSize::Page64K)])
            .unwrap();
    let ctx = mem.fault_context(0x500);
    assert_eq!(ctx.addr, 0x500);
    assert_eq!(ctx.nearest_below, None);
    assert_eq!(ctx.nearest_above, Some("heap"));
}

#[test]
fn fault_context_no_regions_above_returns_none() {
    let mem =
        GuestMemory::from_regions(vec![Region::new(0, 0x100, "heap", PageSize::Page64K)]).unwrap();
    let ctx = mem.fault_context(0x500);
    assert_eq!(ctx.addr, 0x500);
    assert_eq!(ctx.nearest_below, Some("heap"));
    assert_eq!(ctx.nearest_above, None);
}

#[test]
fn containing_region_finds_matching_region() {
    let mem = GuestMemory::from_regions(vec![
        Region::new(0, 0x100, "a", PageSize::Page64K),
        Region::new(0x200, 0x100, "b", PageSize::Page64K),
    ])
    .unwrap();
    assert_eq!(mem.containing_region(0x50, 16).unwrap().label(), "a");
    assert_eq!(mem.containing_region(0x250, 16).unwrap().label(), "b");
    assert!(mem.containing_region(0x150, 16).is_none());
    // Straddling the boundary fails.
    assert!(mem.containing_region(0xF0, 0x20).is_none());
}

#[test]
fn commit_zero_length_is_noop() {
    let mut mem = GuestMemory::new(8);
    mem.apply_commit(range(4, 0), &[]).unwrap();
    assert_eq!(mem.read(range(0, 8)).unwrap(), &[0; 8]);
}

#[test]
fn overlapping_commits_apply_in_call_order() {
    // The commit pipeline guarantees deterministic ordering at a
    // higher level; this test just confirms `apply_commit` itself
    // does not silently buffer or reorder.
    let mut mem = GuestMemory::new(8);
    mem.apply_commit(range(0, 4), &[1, 1, 1, 1]).unwrap();
    mem.apply_commit(range(2, 4), &[2, 2, 2, 2]).unwrap();
    assert_eq!(mem.read(range(0, 8)).unwrap(), &[1, 1, 2, 2, 2, 2, 0, 0]);
}

#[test]
fn content_hash_of_zero_initialized_is_stable() {
    let a = GuestMemory::new(16);
    let b = GuestMemory::new(16);
    assert_eq!(a.content_hash(), b.content_hash());
}

#[test]
fn content_hash_changes_on_commit() {
    let mut mem = GuestMemory::new(8);
    let before = mem.content_hash();
    mem.apply_commit(range(0, 4), &[1, 2, 3, 4]).unwrap();
    let after = mem.content_hash();
    assert_ne!(before, after);
}

#[test]
fn content_hash_is_size_sensitive() {
    // Two zero-initialized buffers of different sizes must hash
    // differently, otherwise replay would mistake a 16-byte zero
    // memory for an 8-byte zero memory.
    let a = GuestMemory::new(8);
    let b = GuestMemory::new(16);
    assert_ne!(a.content_hash(), b.content_hash());
}

#[test]
fn content_hash_is_position_sensitive() {
    // Same bytes at different addresses must produce different
    // hashes; FNV-1a's order dependence guarantees this.
    let mut a = GuestMemory::new(8);
    let mut b = GuestMemory::new(8);
    a.apply_commit(range(0, 1), &[0xff]).unwrap();
    b.apply_commit(range(4, 1), &[0xff]).unwrap();
    assert_ne!(a.content_hash(), b.content_hash());
}

#[test]
fn content_hash_round_trips_after_revert() {
    // Hashing is a pure function of bytes: writing X then writing
    // back the original bytes restores the original hash.
    let mut mem = GuestMemory::new(4);
    let h0 = mem.content_hash();
    mem.apply_commit(range(0, 4), &[1, 2, 3, 4]).unwrap();
    assert_ne!(mem.content_hash(), h0);
    mem.apply_commit(range(0, 4), &[0, 0, 0, 0]).unwrap();
    assert_eq!(mem.content_hash(), h0);
}

#[test]
fn clone_is_independent() {
    let mut a = GuestMemory::new(4);
    a.apply_commit(range(0, 4), &[9, 9, 9, 9]).unwrap();
    let b = a.clone();
    a.apply_commit(range(0, 4), &[0, 0, 0, 0]).unwrap();
    assert_eq!(a.read(range(0, 4)).unwrap(), &[0, 0, 0, 0]);
    assert_eq!(b.read(range(0, 4)).unwrap(), &[9, 9, 9, 9]);
}

#[test]
fn new_constructs_single_region_at_base_zero() {
    let mem = GuestMemory::new(16);
    let regions: Vec<_> = mem.regions().collect();
    assert_eq!(regions.len(), 1);
    assert_eq!(regions[0].base(), 0);
    assert_eq!(regions[0].size(), 16);
    assert_eq!(regions[0].label(), "flat");
}

#[test]
fn from_regions_empty_produces_unmapped_memory() {
    let mem = GuestMemory::from_regions(vec![]).unwrap();
    assert_eq!(mem.size(), 0);
    assert_eq!(mem.read(range(0, 1)), None);
}

#[test]
fn from_regions_single_region_matches_new() {
    let a = GuestMemory::new(32);
    let b = GuestMemory::from_regions(vec![Region::new(0, 32, "flat", PageSize::Page64K)]).unwrap();
    assert_eq!(a.size(), b.size());
    assert_eq!(a.content_hash(), b.content_hash());
}

#[test]
fn from_regions_rejects_overlap_at_base() {
    let err = GuestMemory::from_regions(vec![
        Region::new(0, 0x100, "a", PageSize::Page64K),
        Region::new(0, 0x100, "b", PageSize::Page64K),
    ])
    .unwrap_err();
    assert_eq!(err, MemError::OverlappingRegions);
}

#[test]
fn from_regions_rejects_partial_overlap() {
    // [0, 0x100) and [0x80, 0x180) overlap at [0x80, 0x100).
    let err = GuestMemory::from_regions(vec![
        Region::new(0, 0x100, "a", PageSize::Page64K),
        Region::new(0x80, 0x100, "b", PageSize::Page64K),
    ])
    .unwrap_err();
    assert_eq!(err, MemError::OverlappingRegions);
}

#[test]
fn from_regions_rejects_containment() {
    // [0, 0x200) fully contains [0x80, 0x100).
    let err = GuestMemory::from_regions(vec![
        Region::new(0, 0x200, "big", PageSize::Page64K),
        Region::new(0x80, 0x100, "small", PageSize::Page64K),
    ])
    .unwrap_err();
    assert_eq!(err, MemError::OverlappingRegions);
}

#[test]
fn from_regions_accepts_adjacent_non_overlapping() {
    // [0, 0x100) and [0x100, 0x200) touch but do not overlap.
    let mem = GuestMemory::from_regions(vec![
        Region::new(0, 0x100, "a", PageSize::Page64K),
        Region::new(0x100, 0x100, "b", PageSize::Page64K),
    ])
    .unwrap();
    assert_eq!(mem.regions().count(), 2);
}

#[test]
fn reserved_zero_readable_region_reads_zero_and_bumps_counter() {
    let mem = GuestMemory::from_regions(vec![Region::with_access(
        0xC000_0000,
        0x100,
        "rsx",
        PageSize::Page64K,
        RegionAccess::ReservedZeroReadable,
    )])
    .unwrap();
    assert_eq!(mem.provisional_read_count(), 0);
    let bytes = mem.read(range(0xC000_0000, 8)).unwrap();
    assert_eq!(bytes, &[0u8; 8]);
    assert_eq!(mem.provisional_read_count(), 1);
    // Second read also bumps the counter.
    let _ = mem.read(range(0xC000_0040, 4)).unwrap();
    assert_eq!(mem.provisional_read_count(), 2);
}

#[test]
fn reserved_zero_readable_region_writes_fault() {
    let mut mem = GuestMemory::from_regions(vec![Region::with_access(
        0xC000_0000,
        0x100,
        "rsx",
        PageSize::Page64K,
        RegionAccess::ReservedZeroReadable,
    )])
    .unwrap();
    let err = mem
        .apply_commit(range(0xC000_0000, 4), &[1, 2, 3, 4])
        .unwrap_err();
    assert!(matches!(err, MemError::ReservedWrite { region: "rsx", .. }));
}

#[test]
fn reserved_strict_region_blocks_both_reads_and_writes() {
    let mut mem = GuestMemory::from_regions(vec![Region::with_access(
        0xE000_0000,
        0x100,
        "spu_reserved",
        PageSize::Page64K,
        RegionAccess::ReservedStrict,
    )])
    .unwrap();
    // Reads return None (treated as unmapped from the caller's POV).
    assert_eq!(mem.read(range(0xE000_0000, 4)), None);
    // Writes get the typed reserved-write fault.
    let err = mem
        .apply_commit(range(0xE000_0000, 4), &[1, 2, 3, 4])
        .unwrap_err();
    assert!(matches!(
        err,
        MemError::ReservedWrite {
            region: "spu_reserved",
            ..
        }
    ));
    // Reads do not bump the provisional counter under strict mode.
    assert_eq!(mem.provisional_read_count(), 0);
}

#[test]
fn read_write_region_does_not_bump_provisional_counter() {
    let mem = GuestMemory::new(0x100);
    let _ = mem.read(range(0, 16));
    assert_eq!(mem.provisional_read_count(), 0);
}

#[test]
fn multi_region_read_and_commit_route_by_address() {
    let mut mem = GuestMemory::from_regions(vec![
        Region::new(0, 0x100, "low", PageSize::Page64K),
        Region::new(0x1000, 0x100, "high", PageSize::Page64K),
    ])
    .unwrap();
    mem.apply_commit(range(0x10, 4), &[1, 2, 3, 4]).unwrap();
    mem.apply_commit(range(0x1010, 4), &[9, 9, 9, 9]).unwrap();
    assert_eq!(mem.read(range(0x10, 4)).unwrap(), &[1, 2, 3, 4]);
    assert_eq!(mem.read(range(0x1010, 4)).unwrap(), &[9, 9, 9, 9]);
    // Addresses between the two regions are unmapped.
    assert_eq!(mem.read(range(0x500, 4)), None);
}
