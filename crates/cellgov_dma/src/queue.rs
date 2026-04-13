//! `DmaQueue` -- deterministic priority queue of [`DmaCompletion`]s.
//!
//! Stores completions keyed by `(completion_time, sequence_number)`,
//! where the sequence number is assigned by the queue at enqueue time.
//! That tiebreaker is what makes the order total even when multiple
//! completions land at exactly the same `GuestTicks`, so two runs of
//! the same scenario walk the queue in the same order regardless of
//! host timing or `HashMap` iteration order.
//!
//! Currently only the value type and its draining operations. Wiring
//! `Effect::DmaEnqueue` into the commit pipeline against this queue,
//! and emitting wake events when completions fire, both land as
//! separate slices on top of this seam.
//!
//! Why a [`BTreeMap`] keyed by `(GuestTicks, u64)` rather than a
//! [`std::collections::BinaryHeap`]: replay determinism. Stable ordered
//! collections are required everywhere ordering matters,
//! and `BinaryHeap` does not preserve insertion order among equal-key
//! elements. The `(time, sequence)` `BTreeMap` does, by construction.

use crate::completion::DmaCompletion;
use cellgov_time::GuestTicks;
use std::collections::BTreeMap;

/// A queued entry: the completion metadata plus optional inline data
/// for transfers from unit-private memory.
type QueueEntry = (DmaCompletion, Option<Vec<u8>>);

/// A deterministic priority queue of modeled DMA completions.
///
/// Drains in `(completion_time, sequence)` order. The sequence number
/// is queue-assigned at enqueue time and breaks ties between
/// completions that share a `GuestTicks` value, so the drain order is
/// total and stable across runs.
#[derive(Debug, Clone, Default)]
pub struct DmaQueue {
    entries: BTreeMap<(GuestTicks, u64), QueueEntry>,
    next_seq: u64,
}

impl DmaQueue {
    /// Construct an empty queue.
    #[inline]
    pub fn new() -> Self {
        Self::default()
    }

    /// Number of pending completions.
    #[inline]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the queue holds any completions.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Enqueue `completion` with an optional inline `payload`.
    ///
    /// The queue assigns a fresh sequence number to break ties with any
    /// other completion that shares the same `completion_time`; the
    /// assigned ordering key is the pair
    /// `(completion.completion_time(), sequence_number)`.
    ///
    /// When `payload` is `Some`, the runtime uses those bytes at
    /// completion time instead of reading from the source address in
    /// guest memory. This supports transfers from unit-private memory
    /// (e.g., SPU local store) that is not mapped into the guest
    /// address space.
    ///
    /// Returns the assigned sequence number for callers that want to
    /// trace it; most callers ignore it.
    pub fn enqueue(&mut self, completion: DmaCompletion, payload: Option<Vec<u8>>) -> u64 {
        let seq = self.next_seq;
        self.next_seq += 1;
        self.entries
            .insert((completion.completion_time(), seq), (completion, payload));
        seq
    }

    /// Borrow the earliest pending completion without removing it.
    /// Returns `None` if the queue is empty.
    pub fn peek(&self) -> Option<&DmaCompletion> {
        self.entries.values().next().map(|(c, _)| c)
    }

    /// Remove and return the earliest pending completion and its
    /// payload. Returns `None` if the queue is empty.
    pub fn pop_next(&mut self) -> Option<(DmaCompletion, Option<Vec<u8>>)> {
        let key = *self.entries.keys().next()?;
        self.entries.remove(&key)
    }

    /// Drain every completion whose `completion_time` is less than or
    /// equal to `now`, returning them in `(time, sequence)` order.
    ///
    /// "Less than or equal" because a completion scheduled to fire at
    /// exactly `now` is considered visible at `now`. The relative
    /// order between two completions that share a `completion_time` is
    /// decided by their queue sequence numbers, which preserves
    /// enqueue order.
    pub fn pop_due(&mut self, now: GuestTicks) -> Vec<(DmaCompletion, Option<Vec<u8>>)> {
        match now.raw().checked_add(1) {
            Some(split_time) => {
                let after = self.entries.split_off(&(GuestTicks::new(split_time), 0));
                let due = std::mem::replace(&mut self.entries, after);
                due.into_values().collect()
            }
            None => {
                // now == u64::MAX: everything is due.
                let all = std::mem::take(&mut self.entries);
                all.into_values().collect()
            }
        }
    }
}

#[cfg(test)]
mod tests {
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
        // Three completions all scheduled for the same tick. Drain
        // order must equal enqueue order (sequence number tiebreak).
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
        // Completions scheduled at u64::MAX should still be drained
        // when pop_due is called with u64::MAX. The saturating_add(1)
        // in pop_due must not cause these to be missed.
        let mut q = DmaQueue::new();
        q.enqueue(completion_at(u64::MAX, 0), None);
        q.enqueue(completion_at(u64::MAX - 1, 1), None);
        let due = q.pop_due(GuestTicks::new(u64::MAX));
        assert_eq!(due.len(), 2, "both completions should be drained at MAX");
        assert!(q.is_empty());
    }
}
