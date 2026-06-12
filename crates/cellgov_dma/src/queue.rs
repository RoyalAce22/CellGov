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
use cellgov_time::GuestTicks;
use std::collections::BTreeMap;

/// Completion plus optional inline bytes for transfers from
/// unit-private memory.
type QueueEntry = (DmaCompletion, Option<Vec<u8>>);

/// Deterministic priority queue of modeled DMA completions.
///
/// Drains in `(completion_time, sequence)` order. Sequence is assigned
/// at [`DmaQueue::enqueue`] time.
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
        self.entries
            .insert((completion.completion_time(), seq), (completion, payload));
        seq
    }

    /// Borrow the earliest pending completion without removing it.
    pub fn peek(&self) -> Option<&DmaCompletion> {
        self.entries.values().next().map(|(c, _)| c)
    }

    /// Remove and return the earliest pending completion.
    pub fn pop_next(&mut self) -> Option<(DmaCompletion, Option<Vec<u8>>)> {
        let key = *self.entries.keys().next()?;
        self.entries.remove(&key)
    }

    /// Drain every completion with `completion_time <= now`, in
    /// `(time, sequence)` order.
    pub fn pop_due(&mut self, now: GuestTicks) -> Vec<(DmaCompletion, Option<Vec<u8>>)> {
        match now.raw().checked_add(1) {
            Some(split_time) => {
                let after = self.entries.split_off(&(GuestTicks::new(split_time), 0));
                let due = std::mem::replace(&mut self.entries, after);
                due.into_values().collect()
            }
            None => {
                let all = std::mem::take(&mut self.entries);
                all.into_values().collect()
            }
        }
    }
}

#[cfg(test)]
#[path = "tests/queue_tests.rs"]
mod tests;
