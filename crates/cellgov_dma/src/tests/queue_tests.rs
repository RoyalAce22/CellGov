//! DmaQueue time-ordered draining, enqueue-order tie-breaking, and sequence-number monotonicity.

use super::*;
use crate::request::{DmaDirection, DmaRequest};
use cellgov_event::UnitId;
use cellgov_mem::{ByteRange, GuestAddr};

fn range(start: u64, length: u64) -> ByteRange {
    ByteRange::new(GuestAddr::new(start), length).expect("range fits")
}

fn completion_at(time: u64, issuer: u64) -> DmaCompletion {
    let req = DmaRequest::new(
        DmaDirection::Put,
        range(0x1000, 0x10),
        range(0x9000, 0x10),
        UnitId::new(issuer),
    )
    .unwrap();
    DmaCompletion::new(req, GuestTicks::new(time))
}

#[test]
fn new_queue_is_empty() {
    let q = DmaQueue::new();
    assert!(q.is_empty());
    assert_eq!(q.len(), 0);
    assert!(q.peek().is_none());
}

#[test]
fn enqueue_then_peek_returns_earliest() {
    let mut q = DmaQueue::new();
    q.enqueue(completion_at(100, 0), None);
    q.enqueue(completion_at(50, 1), None);
    q.enqueue(completion_at(200, 2), None);
    assert_eq!(q.len(), 3);
    assert_eq!(q.peek().unwrap().completion_time(), GuestTicks::new(50));
}

#[test]
fn pop_next_returns_in_time_order() {
    let mut q = DmaQueue::new();
    q.enqueue(completion_at(100, 0), None);
    q.enqueue(completion_at(50, 1), None);
    q.enqueue(completion_at(200, 2), None);
    let times: Vec<u64> = std::iter::from_fn(|| q.pop_next())
        .map(|(c, _)| c.completion_time().raw())
        .collect();
    assert_eq!(times, vec![50, 100, 200]);
    assert!(q.is_empty());
}

#[test]
fn pop_next_breaks_ties_by_enqueue_order() {
    let mut q = DmaQueue::new();
    q.enqueue(completion_at(100, 7), None);
    q.enqueue(completion_at(100, 8), None);
    q.enqueue(completion_at(100, 9), None);
    let issuers: Vec<u64> = std::iter::from_fn(|| q.pop_next())
        .map(|(c, _)| c.issuer().raw())
        .collect();
    assert_eq!(issuers, vec![7, 8, 9]);
}

#[test]
fn pop_due_drains_only_due_completions() {
    let mut q = DmaQueue::new();
    q.enqueue(completion_at(50, 0), None);
    q.enqueue(completion_at(100, 1), None);
    q.enqueue(completion_at(150, 2), None);
    q.enqueue(completion_at(200, 3), None);
    let due = q.pop_due(GuestTicks::new(120));
    assert_eq!(due.len(), 2);
    assert_eq!(due[0].0.completion_time(), GuestTicks::new(50));
    assert_eq!(due[1].0.completion_time(), GuestTicks::new(100));
    assert_eq!(q.len(), 2);
    assert_eq!(q.peek().unwrap().completion_time(), GuestTicks::new(150));
}

#[test]
fn pop_due_includes_completions_at_exactly_now() {
    let mut q = DmaQueue::new();
    q.enqueue(completion_at(100, 0), None);
    let due = q.pop_due(GuestTicks::new(100));
    assert_eq!(due.len(), 1);
    assert!(q.is_empty());
}

#[test]
fn pop_due_with_no_due_completions_returns_empty() {
    let mut q = DmaQueue::new();
    q.enqueue(completion_at(500, 0), None);
    let due = q.pop_due(GuestTicks::new(100));
    assert!(due.is_empty());
    assert_eq!(q.len(), 1);
}

#[test]
fn pop_due_on_empty_queue_returns_empty() {
    let mut q = DmaQueue::new();
    let due = q.pop_due(GuestTicks::new(1_000_000));
    assert!(due.is_empty());
}

#[test]
fn pop_due_preserves_enqueue_order_within_same_time() {
    let mut q = DmaQueue::new();
    q.enqueue(completion_at(100, 1), None);
    q.enqueue(completion_at(150, 2), None);
    q.enqueue(completion_at(100, 3), None);
    q.enqueue(completion_at(150, 4), None);
    q.enqueue(completion_at(100, 5), None);
    let due = q.pop_due(GuestTicks::new(200));
    let issuers: Vec<u64> = due.iter().map(|(c, _)| c.issuer().raw()).collect();
    assert_eq!(issuers, vec![1, 3, 5, 2, 4]);
}

#[test]
fn enqueue_returns_sequential_sequence_numbers() {
    let mut q = DmaQueue::new();
    let s0 = q.enqueue(completion_at(100, 0), None);
    let s1 = q.enqueue(completion_at(100, 1), None);
    let s2 = q.enqueue(completion_at(100, 2), None);
    assert_eq!(s0, 0);
    assert_eq!(s1, 1);
    assert_eq!(s2, 2);
}

#[test]
fn drain_then_enqueue_keeps_sequence_strictly_monotonic() {
    let mut q = DmaQueue::new();
    q.enqueue(completion_at(100, 0), None);
    q.enqueue(completion_at(100, 1), None);
    let _ = q.pop_due(GuestTicks::new(100));
    let s2 = q.enqueue(completion_at(100, 2), None);
    assert_eq!(s2, 2);
}

#[test]
fn clone_is_independent() {
    let mut a = DmaQueue::new();
    a.enqueue(completion_at(100, 0), None);
    a.enqueue(completion_at(200, 1), None);
    let mut b = a.clone();
    let _ = a.pop_next();
    assert_eq!(a.len(), 1);
    assert_eq!(b.len(), 2);
    assert_eq!(
        b.pop_next().unwrap().0.completion_time(),
        GuestTicks::new(100)
    );
}

#[test]
fn payload_survives_enqueue_and_pop() {
    let mut q = DmaQueue::new();
    q.enqueue(completion_at(100, 0), Some(vec![0xDE, 0xAD]));
    q.enqueue(completion_at(200, 1), None);
    let (_, payload) = q.pop_next().unwrap();
    assert_eq!(payload, Some(vec![0xDE, 0xAD]));
    let (_, payload) = q.pop_next().unwrap();
    assert_eq!(payload, None);
}

#[test]
fn pop_due_at_max_ticks_drains_everything() {
    let mut q = DmaQueue::new();
    q.enqueue(completion_at(u64::MAX, 0), None);
    q.enqueue(completion_at(u64::MAX - 1, 1), None);
    let due = q.pop_due(GuestTicks::new(u64::MAX));
    assert_eq!(due.len(), 2, "both completions should be drained at MAX");
    assert!(q.is_empty());
}
