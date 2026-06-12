//! DmaCompletion construction, delegating accessors, and equality over request plus time.

use super::*;
use cellgov_mem::GuestAddr;

fn range(start: u64, length: u64) -> ByteRange {
    ByteRange::new(GuestAddr::new(start), length).expect("range fits")
}

fn sample_request() -> DmaRequest {
    DmaRequest::new(
        DmaDirection::Put,
        range(0x1000, 0x40),
        range(0x9000, 0x40),
        UnitId::new(3),
    )
    .expect("equal lengths")
}

#[test]
fn construction_carries_request_and_time() {
    let req = sample_request();
    let c = DmaCompletion::new(req, GuestTicks::new(500));
    assert_eq!(c.request(), req);
    assert_eq!(c.completion_time(), GuestTicks::new(500));
}

#[test]
fn convenience_accessors_delegate_to_request() {
    let req = sample_request();
    let c = DmaCompletion::new(req, GuestTicks::new(0));
    assert_eq!(c.issuer(), UnitId::new(3));
    assert_eq!(c.direction(), DmaDirection::Put);
    assert_eq!(c.source(), range(0x1000, 0x40));
    assert_eq!(c.destination(), range(0x9000, 0x40));
    assert_eq!(c.length(), 0x40);
}

#[test]
fn equality_compares_request_and_time() {
    let req = sample_request();
    let a = DmaCompletion::new(req, GuestTicks::new(100));
    let b = DmaCompletion::new(req, GuestTicks::new(100));
    let c = DmaCompletion::new(req, GuestTicks::new(101));
    assert_eq!(a, b);
    assert_ne!(a, c);
}

#[test]
fn equality_distinguishes_request() {
    let req_a = sample_request();
    let req_b = DmaRequest::new(
        DmaDirection::Get,
        range(0x1000, 0x40),
        range(0x9000, 0x40),
        UnitId::new(3),
    )
    .unwrap();
    let a = DmaCompletion::new(req_a, GuestTicks::new(50));
    let b = DmaCompletion::new(req_b, GuestTicks::new(50));
    assert_ne!(a, b);
}

#[test]
fn copy_semantics_hold() {
    let c = DmaCompletion::new(sample_request(), GuestTicks::new(7));
    let d = c;
    assert_eq!(c, d);
    assert_eq!(c.completion_time(), d.completion_time());
}

#[test]
fn zero_length_completion_is_well_formed() {
    let req = DmaRequest::new(
        DmaDirection::Put,
        range(0x1000, 0),
        range(0x9000, 0),
        UnitId::new(1),
    )
    .unwrap();
    let c = DmaCompletion::new(req, GuestTicks::ZERO);
    assert_eq!(c.length(), 0);
    assert_eq!(c.completion_time(), GuestTicks::ZERO);
}

#[test]
fn completion_time_zero_is_legal() {
    let c = DmaCompletion::new(sample_request(), GuestTicks::ZERO);
    assert_eq!(c.completion_time(), GuestTicks::ZERO);
}
