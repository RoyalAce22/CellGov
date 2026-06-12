//! Ring-cursor wrap, fill tracking, and oldest-first iteration.

use super::*;

#[test]
fn ring_cursor_records_in_order_until_full() {
    let mut c = RingCursor::new(4);
    assert_eq!(c.record(), 0);
    assert_eq!(c.record(), 1);
    assert_eq!(c.filled(), 2);
    assert!(!c.is_full());
}

#[test]
fn ring_cursor_wraps_and_marks_full() {
    let mut c = RingCursor::new(3);
    c.record();
    c.record();
    c.record();
    assert!(c.is_full());
    assert_eq!(c.filled(), 3);
    assert_eq!(c.record(), 0);
    assert_eq!(c.filled(), 3);
    assert!(c.is_full());
}

#[test]
fn ring_cursor_iter_indices_partial_yields_in_order() {
    let mut c = RingCursor::new(4);
    c.record();
    c.record();
    let v: Vec<_> = c.iter_indices().collect();
    assert_eq!(v, vec![0, 1]);
}

#[test]
fn ring_cursor_iter_indices_full_yields_oldest_first() {
    let mut c = RingCursor::new(3);
    for _ in 0..5 {
        c.record();
    }
    let v: Vec<_> = c.iter_indices().collect();
    assert_eq!(v, vec![2, 0, 1]);
}

#[test]
fn ring_cursor_iter_indices_empty_yields_nothing() {
    let c = RingCursor::new(4);
    assert_eq!(c.iter_indices().count(), 0);
}
