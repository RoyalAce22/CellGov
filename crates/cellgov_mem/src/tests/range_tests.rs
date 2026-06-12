//! ByteRange construction, containment, and overlap including u64-boundary cases.

use super::*;

fn r(start: u64, length: u64) -> ByteRange {
    ByteRange::new(GuestAddr::new(start), length).expect("range fits")
}

#[test]
fn construction_basic() {
    let br = r(0x1000, 0x80);
    assert_eq!(br.start(), GuestAddr::new(0x1000));
    assert_eq!(br.length(), 0x80);
    assert_eq!(br.end(), GuestAddr::new(0x1080));
    assert!(!br.is_empty());
}

#[test]
fn empty_range_is_representable() {
    let br = r(0x1000, 0);
    assert!(br.is_empty());
    assert_eq!(br.start(), br.end());
}

#[test]
fn construction_overflow_is_none() {
    let res = ByteRange::new(GuestAddr::new(u64::MAX), 1);
    assert_eq!(res, None);
}

#[test]
fn construction_at_max_zero_length_is_ok() {
    let res = ByteRange::new(GuestAddr::new(u64::MAX), 0);
    assert!(res.is_some());
}

#[test]
fn end_at_u64_max_zero_length() {
    let br = r(u64::MAX, 0);
    assert_eq!(br.end(), GuestAddr::new(u64::MAX));
}

#[test]
fn overlap_near_u64_max() {
    // a = [MAX-0x100, MAX-0x80), b = [MAX-0x80, MAX): adjacent.
    let a = ByteRange::new(GuestAddr::new(u64::MAX - 0x100), 0x80).unwrap();
    let b = ByteRange::new(GuestAddr::new(u64::MAX - 0x80), 0x80).unwrap();
    assert!(!a.overlaps(b));
    // a = [MAX-0x100, MAX-0x80), c = [MAX-0xC0, MAX-0x40): c starts
    // inside a, so they share the bytes [MAX-0xC0, MAX-0x80).
    let c = ByteRange::new(GuestAddr::new(u64::MAX - 0xC0), 0x80).unwrap();
    assert!(a.overlaps(c));
}

#[test]
fn contains_addr_inside() {
    let br = r(0x100, 0x10);
    assert!(br.contains_addr(GuestAddr::new(0x100)));
    assert!(br.contains_addr(GuestAddr::new(0x108)));
    assert!(br.contains_addr(GuestAddr::new(0x10f)));
}

#[test]
fn contains_addr_at_end_is_false() {
    let br = r(0x100, 0x10);
    assert!(!br.contains_addr(GuestAddr::new(0x110)));
}

#[test]
fn contains_addr_below_is_false() {
    let br = r(0x100, 0x10);
    assert!(!br.contains_addr(GuestAddr::new(0xff)));
}

#[test]
fn empty_range_contains_nothing() {
    let br = r(0x100, 0);
    assert!(!br.contains_addr(GuestAddr::new(0x100)));
}

#[test]
fn overlap_overlapping_ranges() {
    let a = r(0x100, 0x20);
    let b = r(0x110, 0x20);
    assert!(a.overlaps(b));
    assert!(b.overlaps(a));
}

#[test]
fn overlap_one_contains_other() {
    let outer = r(0x100, 0x100);
    let inner = r(0x140, 0x10);
    assert!(outer.overlaps(inner));
    assert!(inner.overlaps(outer));
}

#[test]
fn overlap_identical_ranges() {
    let a = r(0x100, 0x20);
    assert!(a.overlaps(a));
}

#[test]
fn overlap_adjacent_is_false() {
    let a = r(0x100, 0x10);
    let b = r(0x110, 0x10);
    assert!(!a.overlaps(b));
    assert!(!b.overlaps(a));
}

#[test]
fn overlap_disjoint_is_false() {
    let a = r(0x100, 0x10);
    let b = r(0x200, 0x10);
    assert!(!a.overlaps(b));
}

#[test]
fn overlap_with_empty_is_false() {
    let a = r(0x100, 0x10);
    let empty = r(0x108, 0);
    assert!(!a.overlaps(empty));
    assert!(!empty.overlaps(a));
}

#[test]
fn overlap_two_empty_is_false() {
    let a = r(0x100, 0);
    let b = r(0x100, 0);
    assert!(!a.overlaps(b));
}

#[test]
fn contiguous_u32_round_trips_basic_inputs() {
    let br = ByteRange::contiguous_u32(0x1000, 0x80);
    assert_eq!(br.start(), GuestAddr::new(0x1000));
    assert_eq!(br.length(), 0x80);
    assert_eq!(br.end(), GuestAddr::new(0x1080));
}

#[test]
fn contiguous_u32_handles_u32_max_endpoints() {
    // u32::MAX + u32::MAX = 0x1_FFFF_FFFE, well below u64::MAX.
    let br = ByteRange::contiguous_u32(u32::MAX, u32::MAX);
    assert_eq!(br.start(), GuestAddr::new(u32::MAX as u64));
    assert_eq!(br.length(), u32::MAX as u64);
    assert_eq!(br.end(), GuestAddr::new(0x1_FFFF_FFFE));
}

#[test]
fn contiguous_u32_zero_length_at_max() {
    let br = ByteRange::contiguous_u32(u32::MAX, 0);
    assert!(br.is_empty());
    assert_eq!(br.end(), GuestAddr::new(u32::MAX as u64));
}
