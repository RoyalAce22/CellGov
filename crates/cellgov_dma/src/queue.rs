//! Deterministic priority queue of [`DmaCompletion`]s.
//!
//! Entries are keyed by `(completion_time, queue-assigned sequence)`,
//! giving a total order that preserves enqueue order among equal times.
//! `Effect::DmaEnqueue` flows through the commit pipeline into this
//! queue; completions emit wake events as they drain.
//
// [CBE-Handbook p:509 s:19] MFC command queues; out-of-order execution; tag-group ordering via fence/barrier.
// [CBE-Handbook p:504 s:18.10.4] 16-entry MFC SPU command queue depth.
// [CBE-Handbook p:522 s:19.3.3.2] 8-entry MFC proxy command queue for PPE-issued commands.

use crate::completion::DmaCompletion;
use cellgov_mem::lanes::{source, LaneMap, LaneValue, ObjectLanes};
use cellgov_time::GuestTicks;

/// Completion plus optional inline bytes for transfers from
/// unit-private memory.
#[derive(Debug, Clone)]
struct QueueEntry {
    completion: DmaCompletion,
    payload: Option<Vec<u8>>,
}

/// Lane fields of one queued completion:
///
/// 1. the completion time
/// 2. the direction, plus 1
/// 3. the source start
/// 4. the destination start
/// 5. the length
/// 6. the issuer
/// 7. the tag's status bit, 0 without a tag
/// 8. 1 when an inline payload is present
/// 9. the payload bytes
impl LaneValue for QueueEntry {
    fn lanes(&self, lanes: &mut ObjectLanes) {
        let c = self.completion;
        lanes.lane(1, 0, c.completion_time().raw());
        lanes.lane(2, 0, c.direction() as u64 + 1);
        lanes.lane(3, 0, c.source().start().raw());
        lanes.lane(4, 0, c.destination().start().raw());
        lanes.lane(5, 0, c.length());
        lanes.lane(6, 0, c.issuer().raw());
        let tag = c.request().tag_id().map_or(0, |tag| tag.status_bit());
        lanes.lane(7, 0, u64::from(tag));
        if let Some(payload) = &self.payload {
            lanes.lane(8, 0, 1);
            lanes.bytes(9, &[], payload);
        }
    }
}

/// Deterministic priority queue of modeled DMA completions.
///
/// Drains in `(completion_time, sequence)` order. Sequence is assigned
/// at [`DmaQueue::enqueue`] time.
#[derive(Debug, Clone)]
pub struct DmaQueue {
    entries: LaneMap<(GuestTicks, u64), QueueEntry>,
    next_seq: u64,
}

impl Default for DmaQueue {
    fn default() -> Self {
        Self {
            entries: LaneMap::new(source::DMA_QUEUE, |(_, seq)| seq),
            next_seq: 0,
        }
    }
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

    /// Enqueue `completion` with optional inline `payload`, returning
    /// the assigned sequence number.
    ///
    /// When `payload` is `Some`, the commit pipeline uses those bytes
    /// at completion time instead of reading from the source address.
    /// This supports transfers from unit-private memory (e.g. SPU local
    /// store) that is not mapped into the guest address space.
    pub fn enqueue(&mut self, completion: DmaCompletion, payload: Option<Vec<u8>>) -> u64 {
        let seq = self.next_seq;
        self.next_seq += 1;
        self.entries.insert(
            (completion.completion_time(), seq),
            QueueEntry {
                completion,
                payload,
            },
        );
        seq
    }

    /// Borrow the earliest pending completion without removing it.
    pub fn peek(&self) -> Option<&DmaCompletion> {
        self.entries.values().next().map(|e| &e.completion)
    }

    /// Every pending completion, in the order the queue drains them,
    /// each with whether an inline payload already carries its bytes.
    ///
    /// A payloaded transfer never reads its source at completion, so a
    /// caller that reasons about what the landing touches needs the
    /// flag as well as the ranges.
    pub fn pending(&self) -> impl Iterator<Item = (&DmaCompletion, bool)> + '_ {
        self.entries
            .values()
            .map(|e| (&e.completion, e.payload.is_some()))
    }

    /// Remove and return the earliest pending completion.
    pub fn pop_next(&mut self) -> Option<(DmaCompletion, Option<Vec<u8>>)> {
        self.entries
            .pop_first()
            .map(|(_, e)| (e.completion, e.payload))
    }

    /// Drain every completion with `completion_time <= now`, in
    /// `(time, sequence)` order.
    pub fn pop_due(&mut self, now: GuestTicks) -> Vec<(DmaCompletion, Option<Vec<u8>>)> {
        let mut due = Vec::new();
        while self
            .entries
            .first()
            .is_some_and(|((time, _), _)| time <= now)
        {
            if let Some((_, e)) = self.entries.pop_first() {
                due.push((e.completion, e.payload));
            }
        }
        due
    }

    /// The queue's partial of the sync-state sum, with the sequence
    /// number of each pending completion as its object.
    #[inline]
    pub fn sync_partial(&self) -> u128 {
        self.entries.partial()
    }

    /// [`Self::sync_partial`] computed from every entry.
    pub fn sync_partial_from_scratch(&self) -> u128 {
        self.entries.partial_from_scratch()
    }
}

#[cfg(test)]
#[path = "tests/queue_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/queue_lanes_tests.rs"]
mod lanes_tests;
