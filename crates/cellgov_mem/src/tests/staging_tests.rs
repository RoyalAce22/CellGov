//! StagingMemory stage/drain ordering and whole-batch rejection on any invalid write.

use super::*;
use crate::addr::GuestAddr;

fn range(start: u64, length: u64) -> ByteRange {
    ByteRange::new(GuestAddr::new(start), length).unwrap()
}

fn staged(start: u64, bytes: &[u8]) -> StagedWrite {
    StagedWrite {
        range: range(start, bytes.len() as u64),
        bytes: bytes.to_vec(),
    }
}

#[test]
fn new_is_empty() {
    let s = StagingMemory::new();
    assert!(s.is_empty());
    assert_eq!(s.len(), 0);
    assert!(s.pending().is_empty());
}

#[test]
fn stage_preserves_order() {
    let mut s = StagingMemory::new();
    s.stage(staged(0, &[1, 1]));
    s.stage(staged(2, &[2, 2]));
    s.stage(staged(4, &[3, 3]));
    assert_eq!(s.len(), 3);
    let starts: Vec<u64> = s.pending().iter().map(|w| w.range.start().raw()).collect();
    assert_eq!(starts, vec![0, 2, 4]);
    s.clear();
}

#[test]
fn clear_discards_everything() {
    let mut s = StagingMemory::new();
    s.stage(staged(0, &[1, 2, 3, 4]));
    s.stage(staged(8, &[5, 6, 7, 8]));
    s.clear();
    assert!(s.is_empty());
}

#[test]
fn drain_into_writes_in_stage_order() {
    let mut mem = GuestMemory::new(16);
    let mut s = StagingMemory::new();
    s.stage(staged(0, &[1, 1, 1, 1]));
    s.stage(staged(8, &[2, 2, 2, 2]));
    let n = s.drain_into(&mut mem).unwrap();
    assert_eq!(n, 2);
    assert!(s.is_empty());
    assert_eq!(
        mem.read(range(0, 16)).unwrap(),
        &[1, 1, 1, 1, 0, 0, 0, 0, 2, 2, 2, 2, 0, 0, 0, 0]
    );
}

#[test]
fn drain_into_overlapping_last_writer_wins_in_stage_order() {
    let mut mem = GuestMemory::new(8);
    let mut s = StagingMemory::new();
    s.stage(staged(0, &[1, 1, 1, 1]));
    s.stage(staged(2, &[2, 2, 2, 2]));
    s.drain_into(&mut mem).unwrap();
    assert_eq!(mem.read(range(0, 8)).unwrap(), &[1, 1, 2, 2, 2, 2, 0, 0]);
}

#[test]
fn drain_into_empty_is_ok_and_noop() {
    let mut mem = GuestMemory::new(4);
    mem.apply_commit(range(0, 4), &[7, 7, 7, 7]).unwrap();
    let mut s = StagingMemory::new();
    let n = s.drain_into(&mut mem).unwrap();
    assert_eq!(n, 0);
    assert_eq!(mem.read(range(0, 4)).unwrap(), &[7, 7, 7, 7]);
}

#[test]
fn drain_into_length_mismatch_rejects_whole_batch_with_neighbors_intact() {
    let mut mem = GuestMemory::new(12);
    let mut s = StagingMemory::new();
    // good, bad, good: validator must stop on the first offender.
    s.stage(staged(0, &[1, 1, 1, 1]));
    s.stage(StagedWrite {
        range: range(4, 4),
        bytes: vec![9, 9],
    });
    s.stage(staged(8, &[2, 2, 2, 2]));
    let err = s.drain_into(&mut mem).unwrap_err();
    assert_eq!(err, MemError::LengthMismatch);
    assert_eq!(mem.read(range(0, 12)).unwrap(), &[0; 12]);
    assert_eq!(s.len(), 3);
    s.clear();
}

#[test]
fn drain_into_out_of_range_rejects_whole_batch() {
    let mut mem = GuestMemory::new(8);
    let mut s = StagingMemory::new();
    s.stage(staged(0, &[1, 1, 1, 1]));
    s.stage(staged(6, &[2, 2, 2, 2]));
    let err = s.drain_into(&mut mem).unwrap_err();
    assert!(matches!(err, MemError::Unmapped(_)));
    assert_eq!(mem.read(range(0, 8)).unwrap(), &[0; 8]);
    assert_eq!(s.len(), 2);
    s.clear();
}

#[test]
fn drain_then_drain_again_is_noop() {
    let mut mem = GuestMemory::new(4);
    let mut s = StagingMemory::new();
    s.stage(staged(0, &[5, 5, 5, 5]));
    s.drain_into(&mut mem).unwrap();
    let n = s.drain_into(&mut mem).unwrap();
    assert_eq!(n, 0);
}

#[test]
fn clone_is_independent() {
    let mut a = StagingMemory::new();
    a.stage(staged(0, &[1, 2, 3]));
    let mut b = a.clone();
    a.clear();
    assert!(a.is_empty());
    assert_eq!(b.len(), 1);
    b.clear();
}

#[test]
fn drain_into_reserved_region_returns_reserved_write() {
    use crate::{PageSize, Region, RegionAccess};
    let mut mem = GuestMemory::from_regions(vec![
        Region::new(0, 256, "main", PageSize::Page64K),
        Region::with_access(
            0xC000_0000,
            256,
            "rsx",
            PageSize::Page64K,
            RegionAccess::ReservedZeroReadable,
        ),
    ])
    .unwrap();
    let mut s = StagingMemory::new();
    s.stage(staged(0xC000_0000, &[1, 2, 3, 4]));
    let err = s.drain_into(&mut mem).unwrap_err();
    assert!(
        matches!(err, MemError::ReservedWrite { region: "rsx", .. }),
        "expected ReservedWrite, got {err:?}"
    );
    s.clear();
}

#[test]
fn drain_into_reserved_strict_region_returns_reserved_write() {
    use crate::{PageSize, Region, RegionAccess};
    let mut mem = GuestMemory::from_regions(vec![
        Region::new(0, 256, "main", PageSize::Page64K),
        Region::with_access(
            0xE000_0000,
            256,
            "spu_reserved",
            PageSize::Page64K,
            RegionAccess::ReservedStrict,
        ),
    ])
    .unwrap();
    let mut s = StagingMemory::new();
    s.stage(staged(0xE000_0000, &[1, 2, 3, 4]));
    let err = s.drain_into(&mut mem).unwrap_err();
    assert!(
        matches!(
            err,
            MemError::ReservedWrite {
                region: "spu_reserved",
                ..
            }
        ),
        "expected ReservedWrite, got {err:?}"
    );
    s.clear();
}

#[test]
fn drain_into_cross_region_span_rejects_whole_batch() {
    use crate::{PageSize, Region};
    let mut mem = GuestMemory::from_regions(vec![
        Region::new(0, 0x100, "main", PageSize::Page64K),
        Region::new(0x100, 0x100, "tail", PageSize::Page64K),
    ])
    .unwrap();
    let mut s = StagingMemory::new();
    // [0xFC, 0x104): straddles the main/tail boundary at 0x100.
    s.stage(staged(0xFC, &[1, 2, 3, 4, 5, 6, 7, 8]));
    let err = s.drain_into(&mut mem).unwrap_err();
    assert!(matches!(err, MemError::Unmapped(_)));
    assert_eq!(s.len(), 1, "rejected batch must remain intact for retry");
    s.clear();
}

#[test]
fn drain_into_zero_length_write_to_rw_region_is_noop() {
    let mut mem = GuestMemory::new(8);
    mem.apply_commit(range(0, 8), &[1, 2, 3, 4, 5, 6, 7, 8])
        .unwrap();
    let mut s = StagingMemory::new();
    s.stage(staged(4, &[]));
    let n = s.drain_into(&mut mem).unwrap();
    assert_eq!(n, 1);
    assert!(s.is_empty());
    assert_eq!(mem.read(range(0, 8)).unwrap(), &[1, 2, 3, 4, 5, 6, 7, 8]);
}

#[test]
fn drain_into_zero_length_write_to_reserved_region_faults() {
    use crate::{PageSize, Region, RegionAccess};
    let mut mem = GuestMemory::from_regions(vec![Region::with_access(
        0xC000_0000,
        256,
        "rsx",
        PageSize::Page64K,
        RegionAccess::ReservedZeroReadable,
    )])
    .unwrap();
    let mut s = StagingMemory::new();
    s.stage(staged(0xC000_0000, &[]));
    let err = s.drain_into(&mut mem).unwrap_err();
    assert!(
        matches!(err, MemError::ReservedWrite { region: "rsx", .. }),
        "expected ReservedWrite, got {err:?}"
    );
    s.clear();
}

#[test]
fn drop_with_pending_writes_panics_in_debug() {
    if cfg!(debug_assertions) {
        let result = std::panic::catch_unwind(|| {
            let mut s = StagingMemory::new();
            s.stage(staged(0, &[1]));
            // s drops here with one pending write.
        });
        assert!(result.is_err(), "expected debug-build panic on leak");
    }
}

impl StagingMemory {
    /// View all pending writes in stage order. Test-only.
    fn pending(&self) -> &[StagedWrite] {
        &self.pending
    }
}
