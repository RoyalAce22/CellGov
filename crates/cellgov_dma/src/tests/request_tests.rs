//! DmaRequest length validation and locked DmaDirection discriminant ordering.

use super::*;
use cellgov_mem::GuestAddr;

fn range(start: u64, length: u64) -> ByteRange {
    ByteRange::new(GuestAddr::new(start), length).expect("range fits")
}

#[test]
fn direction_ordering_is_locked() {
    assert!(DmaDirection::Put < DmaDirection::Get);
    assert_eq!(DmaDirection::Put as u8, 0);
    assert_eq!(DmaDirection::Get as u8, 1);
}

#[test]
fn construction_basic() {
    let req = DmaRequest::new(
        DmaDirection::Put,
        range(0x1000, 0x40),
        range(0x9000, 0x40),
        UnitId::new(3),
    )
    .expect("equal lengths");
    assert_eq!(req.direction(), DmaDirection::Put);
    assert_eq!(req.source(), range(0x1000, 0x40));
    assert_eq!(req.destination(), range(0x9000, 0x40));
    assert_eq!(req.issuer(), UnitId::new(3));
    assert_eq!(req.length(), 0x40);
}

#[test]
fn mismatched_lengths_rejected() {
    let req = DmaRequest::new(
        DmaDirection::Get,
        range(0x1000, 0x40),
        range(0x9000, 0x80),
        UnitId::new(0),
    );
    assert_eq!(req, None);
}

#[test]
fn zero_length_transfer_allowed() {
    let req = DmaRequest::new(
        DmaDirection::Put,
        range(0x1000, 0),
        range(0x9000, 0),
        UnitId::new(1),
    );
    assert!(req.is_some());
    assert_eq!(req.unwrap().length(), 0);
}

#[test]
fn requests_compare_equal_when_fields_match() {
    let a = DmaRequest::new(
        DmaDirection::Get,
        range(0x100, 0x10),
        range(0x200, 0x10),
        UnitId::new(7),
    )
    .unwrap();
    let b = DmaRequest::new(
        DmaDirection::Get,
        range(0x100, 0x10),
        range(0x200, 0x10),
        UnitId::new(7),
    )
    .unwrap();
    assert_eq!(a, b);
}
