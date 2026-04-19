//! Shared waiter-list infrastructure for LV2 synchronization
//! primitives (lwmutex, mutex, semaphore, event queue, event flag,
//! cond).
//!
//! Every primitive parks callers on a FIFO waiter list keyed by the
//! guest-visible primitive id. The wake path dequeues from the head
//! in enqueue order. This is the oracle's determinism contract: the
//! wake order is byte-identical across runs. No priority reordering,
//! no set-based iteration, no host-timed tiebreakers.
//!
//! This module owns the reusable [`WaiterList`] type. Per-primitive
//! tables (`LwMutexTable`, `MutexTable`, etc.) compose `WaiterList`
//! into their entry types.

pub mod cond;
pub mod event_flag;
pub mod event_queue;
pub mod lwmutex;
pub mod mutex;
pub mod semaphore;

pub use cond::{CondEntry, CondTable};
pub use event_flag::{
    EventFlagEntry, EventFlagTable, EventFlagWait, EventFlagWaiter, EventFlagWake,
};
pub use event_queue::{
    EventPayload, EventQueueEntry, EventQueueReceive, EventQueueSend, EventQueueTable,
    EventQueueWaiter,
};
pub use lwmutex::{LwMutexAcquire, LwMutexEntry, LwMutexIdAllocator, LwMutexRelease, LwMutexTable};
pub use mutex::{MutexAcquire, MutexAttrs, MutexEntry, MutexRelease, MutexTable};
pub use semaphore::{SemaphoreEntry, SemaphorePost, SemaphoreTable, SemaphoreWait};

use crate::ppu_thread::PpuThreadId;
use std::collections::VecDeque;

/// FIFO queue of PPU threads parked on a single synchronization
/// primitive.
///
/// Insertion preserves enqueue order. Dequeue always returns the
/// head. Duplicate enqueues of the same `PpuThreadId` are rejected
/// by returning `false` -- a thread cannot be parked on the same
/// primitive twice simultaneously, and the caller is expected to
/// handle that as a programming error. `remove` returns whether the
/// id was present.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WaiterList {
    queue: VecDeque<PpuThreadId>,
}

impl WaiterList {
    /// Construct an empty waiter list.
    pub fn new() -> Self {
        Self {
            queue: VecDeque::new(),
        }
    }

    /// Append `id` to the tail. Returns `true` on success, `false`
    /// if `id` was already present (duplicate-enqueue rejection).
    pub fn enqueue(&mut self, id: PpuThreadId) -> bool {
        if self.queue.iter().any(|&existing| existing == id) {
            return false;
        }
        self.queue.push_back(id);
        true
    }

    /// Pop and return the head, or `None` if empty.
    pub fn dequeue_one(&mut self) -> Option<PpuThreadId> {
        self.queue.pop_front()
    }

    /// Drain all waiters in enqueue order. The list is empty after
    /// the returned iterator is consumed.
    pub fn drain_all(&mut self) -> std::collections::vec_deque::Drain<'_, PpuThreadId> {
        self.queue.drain(..)
    }

    /// Whether `id` is currently parked on this primitive.
    pub fn contains(&self, id: PpuThreadId) -> bool {
        self.queue.iter().any(|&existing| existing == id)
    }

    /// Remove `id` from anywhere in the queue, preserving the
    /// relative order of the remaining waiters. Returns `true` if
    /// `id` was present. Used for targeted wakes
    /// (`sys_cond_signal_to`) and for unwinding a waiter whose
    /// enclosing syscall failed validation after it was enqueued.
    pub fn remove(&mut self, id: PpuThreadId) -> bool {
        if let Some(pos) = self.queue.iter().position(|&existing| existing == id) {
            self.queue.remove(pos);
            true
        } else {
            false
        }
    }

    /// Number of parked waiters.
    pub fn len(&self) -> usize {
        self.queue.len()
    }

    /// Whether the list is empty.
    pub fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }

    /// Iterator over parked waiters in enqueue order. Read-only;
    /// callers that need to wake must go through `dequeue_one` /
    /// `drain_all` / `remove`.
    pub fn iter(&self) -> impl Iterator<Item = PpuThreadId> + '_ {
        self.queue.iter().copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tid(raw: u64) -> PpuThreadId {
        PpuThreadId::new(raw)
    }

    #[test]
    fn empty_list_dequeue_returns_none() {
        let mut w = WaiterList::new();
        assert!(w.is_empty());
        assert_eq!(w.len(), 0);
        assert_eq!(w.dequeue_one(), None);
    }

    #[test]
    fn enqueue_then_dequeue_is_fifo() {
        let mut w = WaiterList::new();
        assert!(w.enqueue(tid(0x0100_0001)));
        assert!(w.enqueue(tid(0x0100_0002)));
        assert!(w.enqueue(tid(0x0100_0003)));
        assert_eq!(w.len(), 3);
        assert_eq!(w.dequeue_one(), Some(tid(0x0100_0001)));
        assert_eq!(w.dequeue_one(), Some(tid(0x0100_0002)));
        assert_eq!(w.dequeue_one(), Some(tid(0x0100_0003)));
        assert_eq!(w.dequeue_one(), None);
    }

    #[test]
    fn duplicate_enqueue_rejected() {
        let mut w = WaiterList::new();
        assert!(w.enqueue(tid(0x0100_0001)));
        assert!(!w.enqueue(tid(0x0100_0001)));
        assert_eq!(w.len(), 1);
    }

    #[test]
    fn drain_all_yields_fifo_and_empties() {
        let mut w = WaiterList::new();
        w.enqueue(tid(0x0100_0001));
        w.enqueue(tid(0x0100_0002));
        w.enqueue(tid(0x0100_0003));
        let drained: Vec<PpuThreadId> = w.drain_all().collect();
        assert_eq!(
            drained,
            vec![tid(0x0100_0001), tid(0x0100_0002), tid(0x0100_0003)],
        );
        assert!(w.is_empty());
    }

    #[test]
    fn contains_tracks_membership() {
        let mut w = WaiterList::new();
        assert!(!w.contains(tid(0x0100_0001)));
        w.enqueue(tid(0x0100_0001));
        assert!(w.contains(tid(0x0100_0001)));
        assert!(!w.contains(tid(0x0100_0002)));
        w.dequeue_one();
        assert!(!w.contains(tid(0x0100_0001)));
    }

    #[test]
    fn remove_preserves_relative_order() {
        let mut w = WaiterList::new();
        w.enqueue(tid(0x0100_0001));
        w.enqueue(tid(0x0100_0002));
        w.enqueue(tid(0x0100_0003));
        assert!(w.remove(tid(0x0100_0002)));
        assert_eq!(w.dequeue_one(), Some(tid(0x0100_0001)));
        assert_eq!(w.dequeue_one(), Some(tid(0x0100_0003)));
    }

    #[test]
    fn remove_missing_returns_false() {
        let mut w = WaiterList::new();
        w.enqueue(tid(0x0100_0001));
        assert!(!w.remove(tid(0x0100_0099)));
        assert_eq!(w.len(), 1);
    }

    #[test]
    fn iter_yields_enqueue_order() {
        let mut w = WaiterList::new();
        w.enqueue(tid(0x0100_0001));
        w.enqueue(tid(0x0100_0002));
        w.enqueue(tid(0x0100_0003));
        let seen: Vec<PpuThreadId> = w.iter().collect();
        assert_eq!(
            seen,
            vec![tid(0x0100_0001), tid(0x0100_0002), tid(0x0100_0003)],
        );
        // iter is read-only; list unchanged.
        assert_eq!(w.len(), 3);
    }

    #[test]
    fn determinism_across_two_runs_of_random_sequence() {
        // Deterministic "random" sequence: xorshift seeded. The list
        // must produce byte-identical outputs across two
        // independent constructions of the same sequence.
        fn run_sequence() -> Vec<Option<PpuThreadId>> {
            let mut w = WaiterList::new();
            let mut state: u64 = 0xDEAD_BEEF_CAFE_F00D;
            let mut trace = Vec::new();
            for _ in 0..256 {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                let op = state & 0b11;
                match op {
                    0 | 1 => {
                        // Enqueue a thread id derived from state.
                        let id = tid(0x0100_0000 | ((state >> 8) & 0xFF));
                        w.enqueue(id);
                    }
                    2 => {
                        trace.push(w.dequeue_one());
                    }
                    _ => {
                        // Remove a specific id if present; records
                        // nothing.
                        let id = tid(0x0100_0000 | ((state >> 16) & 0xFF));
                        w.remove(id);
                    }
                }
            }
            // Drain everything deterministically into trace for
            // comparison.
            while let Some(id) = w.dequeue_one() {
                trace.push(Some(id));
            }
            trace
        }
        let a = run_sequence();
        let b = run_sequence();
        assert_eq!(a, b);
    }
}
