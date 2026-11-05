//! Reads that run across regions sitting end to end: `read` and
//! `read_checked`.

use super::*;
use crate::{ByteRange, GuestAddr};

const BASE: u64 = 0x3000_0000;
const SIZE: usize = 0x1_0000;
const EDGE: u64 = BASE + SIZE as u64;

fn range(addr: u64, len: u64) -> ByteRange {
    ByteRange::new(GuestAddr::new(addr), len).expect("valid range")
}

/// A read-write region at `BASE` and a region of `access` right after it.
fn adjacent(access: RegionAccess) -> GuestMemory {
    GuestMemory::from_regions(vec![
        Region::new(BASE, SIZE, "shm_a", PageSize::Page64K),
        Region::with_access(EDGE, SIZE, "shm_b", PageSize::Page64K, access),
    ])
    .expect("disjoint regions")
}

#[test]
fn a_read_across_two_adjacent_read_write_regions_returns_both_pieces() {
    let mut mem = adjacent(RegionAccess::ReadWrite);
    mem.apply_commit(range(EDGE - 3, 3), &[1, 2, 3]).unwrap();
    mem.apply_commit(range(EDGE, 5), &[4, 5, 6, 7, 8]).unwrap();
    let want = [1, 2, 3, 4, 5, 6, 7, 8];
    assert_eq!(mem.read(range(EDGE - 3, 8)).as_deref(), Some(&want[..]));
    assert_eq!(
        mem.read_checked(range(EDGE - 3, 8)).as_deref(),
        Ok(&want[..])
    );
}

#[test]
fn a_read_inside_one_region_borrows_its_bytes() {
    let mem = adjacent(RegionAccess::ReadWrite);
    assert!(matches!(
        mem.read(range(EDGE - 4, 4)),
        Some(Cow::Borrowed(_))
    ));
}

#[test]
fn a_read_across_a_gap_is_refused_as_unmapped_at_its_start() {
    let mem = GuestMemory::from_regions(vec![
        Region::new(BASE, SIZE, "shm_a", PageSize::Page64K),
        Region::new(EDGE + 0x10, SIZE, "shm_b", PageSize::Page64K),
    ])
    .unwrap();
    assert_eq!(mem.read(range(EDGE - 4, 8)), None);
    assert!(matches!(
        mem.read_checked(range(EDGE - 4, 8)),
        Err(MemError::Unmapped(FaultContext {
            addr: 0x3000_FFFC,
            ..
        }))
    ));
}

#[test]
fn a_read_reaching_a_strict_neighbour_names_its_first_byte_there() {
    let mem = adjacent(RegionAccess::ReservedStrict);
    assert_eq!(mem.read(range(EDGE - 4, 8)), None);
    assert_eq!(
        mem.read_checked(range(EDGE - 4, 8)),
        Err(MemError::ReservedStrictRead {
            addr: EDGE,
            region: "shm_b",
        })
    );
}

#[test]
fn a_read_across_three_regions_takes_the_middle_one_whole() {
    let mut mem = GuestMemory::from_regions(vec![
        Region::new(BASE, SIZE, "shm_a", PageSize::Page64K),
        Region::new(EDGE, 4, "shm_b", PageSize::Page4K),
        Region::new(EDGE + 4, SIZE, "shm_c", PageSize::Page64K),
    ])
    .unwrap();
    mem.apply_commit(range(EDGE - 1, 6), &[1, 2, 3, 4, 5, 6])
        .unwrap();
    assert_eq!(
        mem.read(range(EDGE - 1, 6)).as_deref(),
        Some(&[1, 2, 3, 4, 5, 6][..])
    );
}

#[test]
fn a_read_from_a_zero_readable_region_into_a_strict_one_logs_nothing() {
    let mem = GuestMemory::from_regions(vec![
        Region::with_access(
            BASE,
            SIZE,
            "shm_a",
            PageSize::Page64K,
            RegionAccess::ReservedZeroReadable,
        ),
        Region::with_access(
            EDGE,
            SIZE,
            "shm_b",
            PageSize::Page64K,
            RegionAccess::ReservedStrict,
        ),
    ])
    .unwrap();
    assert_eq!(
        mem.read_checked(range(EDGE - 4, 8)),
        Err(MemError::ReservedStrictRead {
            addr: EDGE,
            region: "shm_b",
        })
    );
    assert_eq!(mem.provisional_read_count(), 0);
}

#[test]
fn a_read_into_a_zero_readable_neighbour_logs_only_that_piece() {
    let mut mem = adjacent(RegionAccess::ReservedZeroReadable);
    mem.apply_commit(range(EDGE - 2, 2), &[7, 7]).unwrap();
    assert_eq!(
        mem.read(range(EDGE - 2, 6)).as_deref(),
        Some(&[7, 7, 0, 0, 0, 0][..])
    );
    assert_eq!(mem.provisional_read_count(), 1);
    let log = mem.drain_provisional_reads();
    assert_eq!(log.len(), 1);
    assert_eq!((log[0].addr, log[0].len, log[0].hits), (EDGE, 4, 1));
}
