//! Writes that run across regions sitting end to end: `validate_write`,
//! `apply_commit` and `holds`.

use super::*;
use crate::{ByteRange, GuestAddr};

const BASE: u64 = 0x3000_0000;
const SIZE: usize = 0x1_0000;

fn range(addr: u64, len: u64) -> ByteRange {
    ByteRange::new(GuestAddr::new(addr), len).expect("valid range")
}

/// Two adjacent read-write regions at `BASE`, as two shared-memory blocks
/// mapped side by side.
fn adjacent() -> GuestMemory {
    let mut mem = GuestMemory::new(4096);
    mem.install_region(BASE, SIZE, "shm_a", PageSize::Page64K)
        .expect("free range");
    mem.install_region(BASE + SIZE as u64, SIZE, "shm_b", PageSize::Page64K)
        .expect("free range");
    mem
}

#[test]
fn a_write_across_two_adjacent_read_write_regions_lands_in_both() {
    let mut mem = adjacent();
    let edge = BASE + SIZE as u64;
    mem.apply_commit(range(edge - 4, 8), &[1, 2, 3, 4, 5, 6, 7, 8])
        .expect("every byte lies in a read-write region");
    assert_eq!(mem.read(range(edge - 4, 4)).unwrap(), &[1, 2, 3, 4]);
    assert_eq!(mem.read(range(edge, 4)).unwrap(), &[5, 6, 7, 8]);

    // The same bytes written one region at a time give the same hash, so
    // the spanning write marked both regions' pages.
    let mut split = adjacent();
    split.content_hash();
    split
        .apply_commit(range(edge - 4, 4), &[1, 2, 3, 4])
        .unwrap();
    split.apply_commit(range(edge, 4), &[5, 6, 7, 8]).unwrap();
    assert_eq!(mem.content_hash(), split.content_hash());
}

#[test]
fn a_write_from_a_read_write_region_into_a_reserved_one_names_the_reserved_regions_first_byte() {
    let mut mem = GuestMemory::new(4096);
    mem.install_region(BASE, SIZE, "shm", PageSize::Page64K)
        .expect("free range");
    let edge = BASE + SIZE as u64;
    mem.install_region_with_access(
        edge,
        SIZE,
        "reserved",
        PageSize::Page64K,
        RegionAccess::ReservedZeroReadable,
    )
    .expect("free range");
    let before = mem.content_hash();
    let err = mem
        .apply_commit(range(edge - 4, 8), &[9; 8])
        .expect_err("the reserved region refuses its half");
    assert_eq!(
        err,
        MemError::ReservedWrite {
            addr: edge,
            region: "reserved"
        }
    );
    assert_eq!(mem.read(range(edge - 4, 4)).unwrap(), &[0; 4]);
    assert_eq!(mem.content_hash(), before);
}

#[test]
fn a_write_running_past_a_region_into_a_gap_is_unmapped_at_its_start() {
    let mut mem = GuestMemory::new(4096);
    mem.install_region(BASE, SIZE, "shm", PageSize::Page64K)
        .expect("free range");
    let start = BASE + SIZE as u64 - 4;
    let err = mem
        .validate_write(range(start, 8), 8)
        .expect_err("half the write lies in no region");
    assert!(
        matches!(err, MemError::Unmapped(ctx) if ctx.addr == start),
        "{err:?}"
    );
}

#[test]
fn a_write_across_a_gap_into_a_later_region_is_unmapped_and_lands_nothing() {
    let mut mem = GuestMemory::new(4096);
    mem.install_region(BASE, SIZE, "shm_a", PageSize::Page64K)
        .expect("free range");
    let gap_end = BASE + SIZE as u64 + 0x1000;
    mem.install_region(gap_end, SIZE, "shm_c", PageSize::Page64K)
        .expect("free range");
    let start = BASE + SIZE as u64 - 4;
    let len = gap_end + 4 - start;
    assert!(!mem.holds(start, len));
    let before = mem.content_hash();
    let err = mem
        .apply_commit(range(start, len), &vec![7; len as usize])
        .expect_err("the gap between the regions holds no byte");
    assert!(matches!(err, MemError::Unmapped(_)), "{err:?}");
    assert_eq!(mem.content_hash(), before);
}

#[test]
fn holds_follows_regions_end_to_end_and_stops_at_a_gap() {
    let mem = adjacent();
    let edge = BASE + SIZE as u64;
    assert!(mem.holds(edge - 4, 8));
    assert!(mem.holds(BASE, 2 * SIZE as u64));
    assert!(!mem.holds(BASE, 2 * SIZE as u64 + 1));
    assert!(!mem.holds(BASE - 1, 2));
    assert!(!mem.holds(edge + SIZE as u64 + 1, 0));
}

#[test]
fn a_write_across_three_adjacent_regions_fills_the_middle_one_whole() {
    let three = || {
        let mut mem = adjacent();
        mem.install_region(BASE + 2 * SIZE as u64, SIZE, "shm_c", PageSize::Page64K)
            .expect("free range");
        mem
    };
    let start = BASE + SIZE as u64 - 4;
    let bytes: Vec<u8> = (0..SIZE + 8).map(|i| (i % 251) as u8 + 1).collect();
    let mut mem = three();
    mem.apply_commit(range(start, bytes.len() as u64), &bytes)
        .expect("every byte lies in a read-write region");
    assert_eq!(mem.read(range(start, 4)).unwrap(), &bytes[..4]);
    assert_eq!(
        mem.read(range(BASE + SIZE as u64, SIZE as u64)).unwrap(),
        &bytes[4..4 + SIZE]
    );
    assert_eq!(
        mem.read(range(BASE + 2 * SIZE as u64, 4)).unwrap(),
        &bytes[4 + SIZE..]
    );

    let mut split = three();
    split.apply_commit(range(start, 4), &bytes[..4]).unwrap();
    split
        .apply_commit(range(BASE + SIZE as u64, SIZE as u64), &bytes[4..4 + SIZE])
        .unwrap();
    split
        .apply_commit(range(BASE + 2 * SIZE as u64, 4), &bytes[4 + SIZE..])
        .unwrap();
    assert_eq!(mem.content_hash(), split.content_hash());
    assert_eq!(mem.content_hash(), mem.content_hash_from_scratch());
}
