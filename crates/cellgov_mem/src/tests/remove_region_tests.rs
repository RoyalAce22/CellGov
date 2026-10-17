//! `GuestMemory::remove_region`: the exact-match rule, the address that
//! faults once its region is gone, and the content hash after removal.

use super::*;
use crate::{ByteRange, GuestAddr};

const BASE: u64 = 0x5000_0000;
const SIZE: usize = 0x1_0000;

fn with_window() -> GuestMemory {
    let mut mem = GuestMemory::new(4096);
    mem.install_region(BASE, SIZE, "shm", PageSize::Page64K)
        .expect("free range");
    mem
}

fn range(addr: u64, len: u64) -> ByteRange {
    ByteRange::new(GuestAddr::new(addr), len).expect("valid range")
}

#[test]
fn a_removed_region_is_unmapped_and_the_hash_is_the_one_before_its_install() {
    let mut mem = GuestMemory::new(4096);
    let before = mem.content_hash();
    mem.install_region(BASE, SIZE, "shm", PageSize::Page64K)
        .expect("free range");
    mem.apply_commit(range(BASE + 0x10, 4), &[1, 2, 3, 4])
        .expect("mapped");
    assert_ne!(mem.content_hash(), before);

    mem.remove_region(BASE, SIZE).expect("the region exists");

    assert_eq!(mem.read(range(BASE + 0x10, 4)), None);
    assert_eq!(mem.regions().count(), 1);
    assert_eq!(mem.content_hash(), before);
    assert_eq!(mem.content_hash(), mem.content_hash_from_scratch());
}

#[test]
fn a_removal_names_the_exact_base_and_size() {
    let mut mem = with_window();
    for (base, size) in [
        (BASE + 0x1000, SIZE - 0x1000),
        (BASE, SIZE / 2),
        (0x9000, SIZE),
    ] {
        assert_eq!(
            mem.remove_region(base, size),
            Err(MemError::NoSuchRegion {
                base,
                size: size as u64
            }),
            "{base:#x}+{size:#x}"
        );
    }
    assert_eq!(mem.regions().count(), 2, "nothing was removed");
    mem.remove_region(BASE, SIZE).expect("exact match");
    assert_eq!(
        mem.remove_region(BASE, SIZE),
        Err(MemError::NoSuchRegion {
            base: BASE,
            size: SIZE as u64
        }),
        "a second removal finds nothing"
    );
}

#[test]
fn a_removed_range_takes_a_fresh_zeroed_install() {
    let mut mem = with_window();
    mem.apply_commit(range(BASE, 4), &[9, 9, 9, 9])
        .expect("mapped");
    mem.remove_region(BASE, SIZE).expect("the region exists");
    mem.install_region(BASE, SIZE, "shm", PageSize::Page64K)
        .expect("the range is free again");
    assert_eq!(mem.read(range(BASE, 4)), Some(&[0u8, 0, 0, 0][..]));
    assert_eq!(mem.content_hash(), mem.content_hash_from_scratch());
}

#[test]
fn a_region_after_a_removed_one_keeps_its_own_page_terms() {
    let mut mem = with_window();
    let high = BASE + 0x100_0000;
    mem.install_region(high, SIZE, "shm", PageSize::Page64K)
        .expect("free range");
    mem.apply_commit(range(high + 0x20, 4), &[5, 6, 7, 8])
        .expect("mapped");
    let _ = mem.content_hash();
    mem.remove_region(BASE, SIZE).expect("the region exists");
    mem.apply_commit(range(high + 0x8000, 4), &[1, 1, 1, 1])
        .expect("mapped");
    mem.invalidate_content_hash();
    assert_eq!(mem.content_hash(), mem.content_hash_from_scratch());
}
