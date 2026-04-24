//! Per-primitive state tables and the shared FIFO waiter list.
//!
//! Wake order is strictly FIFO by enqueue order.

pub mod cond;
pub mod event_flag;
pub mod event_queue;
pub mod lwmutex;
pub mod mutex;
pub mod semaphore;

pub use cond::{CondCreateError, CondEnqueueError, CondEntry, CondSignalToError, CondTable};
pub use event_flag::{
    EventFlagCreateError, EventFlagEnqueueError, EventFlagEntry, EventFlagTable, EventFlagWait,
    EventFlagWaiter, EventFlagWake,
};
pub use event_queue::{
    EventPayload, EventQueueEnqueueError, EventQueueEntry, EventQueueReceive, EventQueueSend,
    EventQueueTable, EventQueueWaiter,
};
pub use lwmutex::{
    LwMutexAcquire, LwMutexAcquireOrEnqueue, LwMutexEnqueueError, LwMutexEntry, LwMutexIdAllocator,
    LwMutexRelease, LwMutexTable,
};
pub use mutex::{
    MutexAcquire, MutexAcquireOrEnqueue, MutexAttrs, MutexCreateError, MutexEnqueueError,
    MutexEntry, MutexRelease, MutexTable,
};
pub use semaphore::{
    SemaphoreCreateError, SemaphoreEnqueueError, SemaphoreEntry, SemaphorePost, SemaphoreTable,
    SemaphoreWait,
};

use crate::ppu_thread::PpuThreadId;
use std::collections::VecDeque;

/// [`WaiterList::enqueue`] rejection: `id` was already parked.
///
/// Callers must route this to `record_invariant_break`; ignoring
/// it drops the second wait's `PendingResponse`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DuplicateEnqueue {
    /// Thread id that was already parked.
    pub id: PpuThreadId,
}

/// FIFO queue of PPU threads parked on a single primitive.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WaiterList {
    queue: VecDeque<PpuThreadId>,
}

impl WaiterList {
    /// Construct an empty list.
    pub fn new() -> Self {
        Self {
            queue: VecDeque::new(),
        }
    }

    /// Append `id` to the tail.
    ///
    /// # Errors
    /// [`DuplicateEnqueue`] if `id` is already parked.
    #[must_use = "duplicate enqueue is a programming error; the caller must handle it"]
    pub fn enqueue(&mut self, id: PpuThreadId) -> Result<(), DuplicateEnqueue> {
        if self.queue.iter().any(|&existing| existing == id) {
            return Err(DuplicateEnqueue { id });
        }
        self.queue.push_back(id);
        Ok(())
    }

    /// Pop the head, or `None` if empty.
    pub fn dequeue_one(&mut self) -> Option<PpuThreadId> {
        self.queue.pop_front()
    }

    /// Drain all waiters in enqueue order.
    pub fn drain_all(&mut self) -> std::collections::vec_deque::Drain<'_, PpuThreadId> {
        self.queue.drain(..)
    }

    /// Whether `id` is currently parked.
    pub fn contains(&self, id: PpuThreadId) -> bool {
        self.queue.iter().any(|&existing| existing == id)
    }

    /// Remove `id`, preserving the order of the remaining waiters.
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

    /// Read-only iterator in enqueue order.
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
        w.enqueue(tid(0x0100_0001)).unwrap();
        w.enqueue(tid(0x0100_0002)).unwrap();
        w.enqueue(tid(0x0100_0003)).unwrap();
        assert_eq!(w.len(), 3);
        assert_eq!(w.dequeue_one(), Some(tid(0x0100_0001)));
        assert_eq!(w.dequeue_one(), Some(tid(0x0100_0002)));
        assert_eq!(w.dequeue_one(), Some(tid(0x0100_0003)));
        assert_eq!(w.dequeue_one(), None);
    }

    #[test]
    fn duplicate_enqueue_rejected() {
        let mut w = WaiterList::new();
        w.enqueue(tid(0x0100_0001)).unwrap();
        assert_eq!(
            w.enqueue(tid(0x0100_0001)),
            Err(DuplicateEnqueue {
                id: tid(0x0100_0001)
            })
        );
        assert_eq!(w.len(), 1);
    }

    #[test]
    fn drain_all_yields_fifo_and_empties() {
        let mut w = WaiterList::new();
        w.enqueue(tid(0x0100_0001)).unwrap();
        w.enqueue(tid(0x0100_0002)).unwrap();
        w.enqueue(tid(0x0100_0003)).unwrap();
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
        w.enqueue(tid(0x0100_0001)).unwrap();
        assert!(w.contains(tid(0x0100_0001)));
        assert!(!w.contains(tid(0x0100_0002)));
        w.dequeue_one();
        assert!(!w.contains(tid(0x0100_0001)));
    }

    #[test]
    fn remove_preserves_relative_order() {
        let mut w = WaiterList::new();
        w.enqueue(tid(0x0100_0001)).unwrap();
        w.enqueue(tid(0x0100_0002)).unwrap();
        w.enqueue(tid(0x0100_0003)).unwrap();
        assert!(w.remove(tid(0x0100_0002)));
        assert_eq!(w.dequeue_one(), Some(tid(0x0100_0001)));
        assert_eq!(w.dequeue_one(), Some(tid(0x0100_0003)));
    }

    #[test]
    fn remove_missing_returns_false() {
        let mut w = WaiterList::new();
        w.enqueue(tid(0x0100_0001)).unwrap();
        assert!(!w.remove(tid(0x0100_0099)));
        assert_eq!(w.len(), 1);
    }

    #[test]
    fn iter_yields_enqueue_order() {
        let mut w = WaiterList::new();
        w.enqueue(tid(0x0100_0001)).unwrap();
        w.enqueue(tid(0x0100_0002)).unwrap();
        w.enqueue(tid(0x0100_0003)).unwrap();
        let seen: Vec<PpuThreadId> = w.iter().collect();
        assert_eq!(
            seen,
            vec![tid(0x0100_0001), tid(0x0100_0002), tid(0x0100_0003)],
        );
        assert_eq!(w.len(), 3);
    }

    /// Byte trace of a fixed xorshift sequence: `E`+id on
    /// enqueue-ok, `e` on rejection, `D`+id on dequeue, `d` on
    /// empty, `R`/`r` on remove hit/miss, `F`+id on final drain.
    fn determinism_trace() -> Vec<u8> {
        let mut w = WaiterList::new();
        let mut state: u64 = 0xDEAD_BEEF_CAFE_F00D;
        let mut trace = Vec::new();
        for _ in 0..256 {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            match state & 0b11 {
                0 | 1 => {
                    let id = tid(0x0100_0000 | ((state >> 8) & 0xFF));
                    match w.enqueue(id) {
                        Ok(()) => {
                            trace.push(b'E');
                            trace.extend_from_slice(&id.raw().to_le_bytes());
                        }
                        Err(_) => trace.push(b'e'),
                    }
                }
                2 => match w.dequeue_one() {
                    Some(id) => {
                        trace.push(b'D');
                        trace.extend_from_slice(&id.raw().to_le_bytes());
                    }
                    None => trace.push(b'd'),
                },
                _ => {
                    let id = tid(0x0100_0000 | ((state >> 16) & 0xFF));
                    trace.push(if w.remove(id) { b'R' } else { b'r' });
                }
            }
        }
        while let Some(id) = w.dequeue_one() {
            trace.push(b'F');
            trace.extend_from_slice(&id.raw().to_le_bytes());
        }
        trace
    }

    #[test]
    fn determinism_across_two_runs_of_random_sequence() {
        const EXPECTED_LEN: usize = 1802;
        const EXPECTED_HASH: u64 = 0x35DB_8B6B_AF21_EC62;
        let trace = determinism_trace();
        assert_eq!(trace.len(), EXPECTED_LEN, "trace length drifted");
        let mut hasher = cellgov_mem::Fnv1aHasher::new();
        hasher.write(&trace);
        assert_eq!(
            hasher.finish(),
            EXPECTED_HASH,
            "trace content drifted; update EXPECTED_HASH only after auditing the change",
        );
        assert_eq!(trace, determinism_trace());
    }
}
