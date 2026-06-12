//! GuestTicks checked/saturating arithmetic, duration-since, and ordering at the u64 boundary.

use super::*;

#[test]
fn zero_is_origin() {
    assert_eq!(GuestTicks::ZERO.raw(), 0);
    assert_eq!(GuestTicks::ZERO, GuestTicks::new(0));
}

#[test]
fn ordering_is_total_and_monotonic() {
    let a = GuestTicks::new(5);
    let b = GuestTicks::new(10);
    assert!(a < b);
    assert!(b > a);
    assert_eq!(a, GuestTicks::new(5));
}

#[test]
fn ordering_holds_at_max_boundary() {
    assert!(GuestTicks::new(u64::MAX - 1) < GuestTicks::new(u64::MAX));
}

#[test]
fn checked_add_advances() {
    let t = GuestTicks::new(100);
    assert_eq!(
        t.checked_add(GuestTicks::new(25)),
        Some(GuestTicks::new(125))
    );
}

#[test]
fn checked_add_overflows_to_none() {
    let t = GuestTicks::new(u64::MAX);
    assert_eq!(t.checked_add(GuestTicks::new(1)), None);
}

#[test]
fn saturating_add_pins_at_max() {
    let t = GuestTicks::new(u64::MAX - 3);
    assert_eq!(
        t.saturating_add(GuestTicks::new(10)),
        GuestTicks::new(u64::MAX)
    );
}

#[test]
fn duration_since_earlier_is_some() {
    let earlier = GuestTicks::new(7);
    let later = GuestTicks::new(20);
    assert_eq!(
        later.checked_duration_since(earlier),
        Some(GuestTicks::new(13))
    );
}

#[test]
fn duration_since_future_is_none() {
    let earlier = GuestTicks::new(20);
    let later = GuestTicks::new(7);
    assert_eq!(later.checked_duration_since(earlier), None);
}

#[test]
fn new_raw_round_trip() {
    for v in [0u64, 1, 2, 41, 42, 0x1_0000, u64::MAX - 1, u64::MAX] {
        assert_eq!(GuestTicks::new(v).raw(), v);
    }
}

#[test]
fn default_is_zero() {
    assert_eq!(GuestTicks::default(), GuestTicks::ZERO);
}

#[test]
fn display_is_bare_number() {
    assert_eq!(format!("{}", GuestTicks::ZERO), "0");
    assert_eq!(format!("{}", GuestTicks::new(42)), "42");
    assert_eq!(
        format!("{}", GuestTicks::new(u64::MAX)),
        format!("{}", u64::MAX),
    );
}
