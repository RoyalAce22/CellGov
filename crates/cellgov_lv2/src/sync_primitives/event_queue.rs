//! Event queue table.
//!
//! Owns the state for `sys_event_queue_create` / `_destroy` /
//! `_receive` / `_send` / `_tryreceive`. Each entry holds a FIFO
//! `VecDeque<EventPayload>` of buffered payloads and a FIFO
//! list of `EventQueueWaiter` entries (each carries the thread
//! id plus the receive-side out pointer the wait handler stored
//! at park time).
//!
//! Observable contract on send:
//!
//!   "A payload arrives at the queue; one waiter (if any)
//!   receives it from the queue in send-arrival order."
//!
//! The implementation fast-paths the handoff: if a waiter is
//! parked, `send_and_wake_or_enqueue` hands the payload directly
//! to that waiter. Tests assert the observable result (payload
//! content, wake ordering, r3 value), not a specific storage
//! path; the fast-path and the queue-then-pop path are
//! indistinguishable from the guest's perspective.

use crate::ppu_thread::PpuThreadId;
use std::collections::{BTreeMap, VecDeque};

/// One queued event. Matches the PS3 `sys_event_t` layout -- four
/// u64 fields the caller's `receive` out-pointer is populated
/// with when the event is delivered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EventPayload {
    /// Source id. Typically the sending event-port id or a
    /// user-defined tag.
    pub source: u64,
    /// First data word.
    pub data1: u64,
    /// Second data word.
    pub data2: u64,
    /// Third data word.
    pub data3: u64,
}

/// Outcome of a `try_receive` call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventQueueReceive {
    /// A payload was available and has been popped; deliver to
    /// the caller's out pointer with r3 = 0.
    Delivered(EventPayload),
    /// Queue empty and no waiter parked -- caller should park
    /// (for `_receive`) or return ENOENT (for `_tryreceive`).
    Empty,
}

/// Outcome of a `send_and_wake_or_enqueue` call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventQueueSend {
    /// A parked waiter received the payload directly. Runtime
    /// should wake `new_owner` and deliver the payload to
    /// `out_ptr` via `PendingResponse::EventQueueReceive`.
    Woke {
        /// Thread that received the payload.
        new_owner: PpuThreadId,
        /// Out pointer the waiter supplied at wait time.
        out_ptr: u32,
        /// Payload to deliver on wake.
        payload: EventPayload,
    },
    /// No waiters; the payload was enqueued.
    Enqueued,
    /// Queue full (payload count would exceed the configured
    /// size). Caller returns EBUSY.
    Full,
    /// Unknown queue id. Caller returns ESRCH.
    Unknown,
}

/// A parked `sys_event_queue_receive` caller.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EventQueueWaiter {
    /// Thread parked on the queue.
    pub thread: PpuThreadId,
    /// Guest address of the `sys_event_t` output buffer.
    pub out_ptr: u32,
}

/// A single event queue.
///
/// The waiter list is a `VecDeque<EventQueueWaiter>` rather than
/// the shared `WaiterList` because each parked caller needs its
/// own `out_ptr`. Co-locating the continuation pointer with the
/// thread id lets the send side emit a complete
/// `PendingResponse::EventQueueReceive` at wake time with no
/// merge-with-previous-response logic. See the
/// "WaiterContinuation" design note in `sync_primitives::mod`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventQueueEntry {
    payloads: VecDeque<EventPayload>,
    waiters: VecDeque<EventQueueWaiter>,
    size: u32,
}

impl EventQueueEntry {
    fn new(size: u32) -> Self {
        Self {
            payloads: VecDeque::new(),
            waiters: VecDeque::new(),
            size,
        }
    }

    /// Maximum number of payloads the queue can buffer.
    pub fn size(&self) -> u32 {
        self.size
    }

    /// Current number of buffered payloads.
    pub fn len(&self) -> usize {
        self.payloads.len()
    }

    /// Whether no payloads are currently buffered.
    pub fn is_empty(&self) -> bool {
        self.payloads.is_empty()
    }

    /// Read-only view of the waiter list.
    pub fn waiters(&self) -> &VecDeque<EventQueueWaiter> {
        &self.waiters
    }

    /// Whether no threads are currently parked on receive.
    pub fn has_no_waiters(&self) -> bool {
        self.waiters.is_empty()
    }

    /// Read-only iterator over buffered payloads in arrival order.
    pub fn iter(&self) -> impl Iterator<Item = &EventPayload> {
        self.payloads.iter()
    }
}

/// Table of event queues.
#[derive(Debug, Clone, Default)]
pub struct EventQueueTable {
    entries: BTreeMap<u32, EventQueueEntry>,
}

impl EventQueueTable {
    /// Construct an empty table.
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert a fresh entry with the given id and max size.
    /// Returns `false` if `id` is already present or `size == 0`.
    pub fn create_with_id(&mut self, id: u32, size: u32) -> bool {
        if self.entries.contains_key(&id) || size == 0 {
            return false;
        }
        self.entries.insert(id, EventQueueEntry::new(size));
        true
    }

    /// Destroy an event queue. Returns the removed entry or
    /// `None` if the id was unknown.
    pub fn destroy(&mut self, id: u32) -> Option<EventQueueEntry> {
        self.entries.remove(&id)
    }

    /// Read-only lookup.
    pub fn lookup(&self, id: u32) -> Option<&EventQueueEntry> {
        self.entries.get(&id)
    }

    /// Number of tracked queues.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the table has no entries.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Try to pop a payload for the caller.
    ///
    /// Returns `None` if `id` is unknown, `Some(Delivered)` if a
    /// payload was popped, `Some(Empty)` if the queue is empty.
    pub fn try_receive(&mut self, id: u32) -> Option<EventQueueReceive> {
        let entry = self.entries.get_mut(&id)?;
        match entry.payloads.pop_front() {
            Some(p) => Some(EventQueueReceive::Delivered(p)),
            None => Some(EventQueueReceive::Empty),
        }
    }

    /// Non-blocking batch pop: drain up to `max` payloads from
    /// `id` in arrival order. Returns `None` if `id` is unknown.
    /// Returned `Vec` is empty when the queue had no payloads.
    pub fn try_receive_batch(&mut self, id: u32, max: usize) -> Option<Vec<EventPayload>> {
        let entry = self.entries.get_mut(&id)?;
        let n = entry.payloads.len().min(max);
        let mut out = Vec::with_capacity(n);
        for _ in 0..n {
            out.push(entry.payloads.pop_front().expect("pop_front within len"));
        }
        Some(out)
    }

    /// Enqueue `waiter` on the receive-waiter list for `id`.
    /// Returns `true` on success, `false` if `id` is unknown or
    /// `thread` is already parked on this queue.
    pub fn enqueue_waiter(&mut self, id: u32, thread: PpuThreadId, out_ptr: u32) -> bool {
        let Some(entry) = self.entries.get_mut(&id) else {
            return false;
        };
        if entry.waiters.iter().any(|w| w.thread == thread) {
            return false;
        }
        entry
            .waiters
            .push_back(EventQueueWaiter { thread, out_ptr });
        true
    }

    /// Send a payload into `id`.
    ///
    /// If a waiter is parked, the payload is handed directly to
    /// the head of the waiter list; the returned `Woke` variant
    /// carries both the waiter's thread id AND its recorded
    /// `out_ptr` so the send-side dispatch can build a complete
    /// `PendingResponse::EventQueueReceive` without consulting
    /// the runtime's syscall-response table. Otherwise the
    /// payload is enqueued; if the queue is already at `size`
    /// items, the send fails with `Full`.
    pub fn send_and_wake_or_enqueue(&mut self, id: u32, payload: EventPayload) -> EventQueueSend {
        let Some(entry) = self.entries.get_mut(&id) else {
            return EventQueueSend::Unknown;
        };
        match entry.waiters.pop_front() {
            Some(waiter) => EventQueueSend::Woke {
                new_owner: waiter.thread,
                out_ptr: waiter.out_ptr,
                payload,
            },
            None => {
                if (entry.payloads.len() as u32) >= entry.size {
                    EventQueueSend::Full
                } else {
                    entry.payloads.push_back(payload);
                    EventQueueSend::Enqueued
                }
            }
        }
    }

    /// FNV-1a digest of the table for state-hash folding.
    pub fn state_hash(&self) -> u64 {
        let mut hasher = cellgov_mem::Fnv1aHasher::new();
        hasher.write(&(self.entries.len() as u64).to_le_bytes());
        for (id, entry) in &self.entries {
            hasher.write(&id.to_le_bytes());
            hasher.write(&entry.size.to_le_bytes());
            hasher.write(&(entry.payloads.len() as u64).to_le_bytes());
            for p in entry.payloads.iter() {
                hasher.write(&p.source.to_le_bytes());
                hasher.write(&p.data1.to_le_bytes());
                hasher.write(&p.data2.to_le_bytes());
                hasher.write(&p.data3.to_le_bytes());
            }
            hasher.write(&(entry.waiters.len() as u64).to_le_bytes());
            for w in entry.waiters.iter() {
                hasher.write(&w.thread.raw().to_le_bytes());
                // Include out_ptr so two tables differing only
                // in the parked waiter's continuation pointer
                // produce distinct hashes. Foundation titles
                // never park event-queue waiters, so this does
                // not affect their hash -- the table is gated
                // empty-unless-used at the host level.
                hasher.write(&w.out_ptr.to_le_bytes());
            }
        }
        hasher.finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tid(raw: u64) -> PpuThreadId {
        PpuThreadId::new(raw)
    }

    fn pl(source: u64) -> EventPayload {
        EventPayload {
            source,
            data1: source + 1,
            data2: source + 2,
            data3: source + 3,
        }
    }

    #[test]
    fn fresh_table_is_empty() {
        let t = EventQueueTable::new();
        assert!(t.is_empty());
    }

    #[test]
    fn create_rejects_zero_size() {
        let mut t = EventQueueTable::new();
        assert!(!t.create_with_id(1, 0));
        assert!(t.lookup(1).is_none());
    }

    #[test]
    fn create_rejects_collision() {
        let mut t = EventQueueTable::new();
        assert!(t.create_with_id(1, 4));
        assert!(!t.create_with_id(1, 4));
    }

    #[test]
    fn try_receive_empty_queue_returns_empty() {
        let mut t = EventQueueTable::new();
        t.create_with_id(1, 4);
        assert_eq!(t.try_receive(1), Some(EventQueueReceive::Empty));
    }

    #[test]
    fn try_receive_unknown_is_none() {
        let mut t = EventQueueTable::new();
        assert!(t.try_receive(99).is_none());
    }

    #[test]
    fn send_with_no_waiters_enqueues_and_try_receive_returns_it() {
        let mut t = EventQueueTable::new();
        t.create_with_id(1, 4);
        assert_eq!(
            t.send_and_wake_or_enqueue(1, pl(10)),
            EventQueueSend::Enqueued
        );
        assert_eq!(t.try_receive(1), Some(EventQueueReceive::Delivered(pl(10))),);
        // Queue back to empty.
        assert_eq!(t.try_receive(1), Some(EventQueueReceive::Empty));
    }

    #[test]
    fn send_preserves_arrival_order_across_multiple_payloads() {
        let mut t = EventQueueTable::new();
        t.create_with_id(1, 4);
        assert_eq!(
            t.send_and_wake_or_enqueue(1, pl(1)),
            EventQueueSend::Enqueued
        );
        assert_eq!(
            t.send_and_wake_or_enqueue(1, pl(2)),
            EventQueueSend::Enqueued
        );
        assert_eq!(
            t.send_and_wake_or_enqueue(1, pl(3)),
            EventQueueSend::Enqueued
        );
        assert_eq!(t.try_receive(1), Some(EventQueueReceive::Delivered(pl(1))),);
        assert_eq!(t.try_receive(1), Some(EventQueueReceive::Delivered(pl(2))),);
        assert_eq!(t.try_receive(1), Some(EventQueueReceive::Delivered(pl(3))),);
    }

    #[test]
    fn send_with_parked_waiter_hands_off_directly_and_does_not_enqueue() {
        // The observable contract: on send, exactly one waiter
        // receives the payload, the queue storage is unchanged
        // (the payload went directly to the woken waiter).
        let mut t = EventQueueTable::new();
        t.create_with_id(1, 4);
        t.enqueue_waiter(1, tid(0x0100_0001), 0x2000);
        assert_eq!(
            t.send_and_wake_or_enqueue(1, pl(42)),
            EventQueueSend::Woke {
                new_owner: tid(0x0100_0001),
                out_ptr: 0x2000,
                payload: pl(42),
            },
        );
        // Queue storage is empty (payload was fast-pathed to the
        // waiter); waiter list drained.
        assert!(t.lookup(1).unwrap().is_empty());
        assert!(t.lookup(1).unwrap().waiters().is_empty());
    }

    #[test]
    fn send_wakes_waiters_in_fifo_order() {
        let mut t = EventQueueTable::new();
        t.create_with_id(1, 4);
        t.enqueue_waiter(1, tid(0x0100_0001), 0x2000);
        t.enqueue_waiter(1, tid(0x0100_0002), 0x2020);
        t.enqueue_waiter(1, tid(0x0100_0003), 0x2040);
        // First send wakes w1.
        assert_eq!(
            t.send_and_wake_or_enqueue(1, pl(10)),
            EventQueueSend::Woke {
                new_owner: tid(0x0100_0001),
                out_ptr: 0x2000,
                payload: pl(10),
            },
        );
        // Second send wakes w2.
        assert_eq!(
            t.send_and_wake_or_enqueue(1, pl(20)),
            EventQueueSend::Woke {
                new_owner: tid(0x0100_0002),
                out_ptr: 0x2020,
                payload: pl(20),
            },
        );
        // Third send wakes w3.
        assert_eq!(
            t.send_and_wake_or_enqueue(1, pl(30)),
            EventQueueSend::Woke {
                new_owner: tid(0x0100_0003),
                out_ptr: 0x2040,
                payload: pl(30),
            },
        );
        // No more waiters -- next send enqueues.
        assert_eq!(
            t.send_and_wake_or_enqueue(1, pl(40)),
            EventQueueSend::Enqueued
        );
    }

    #[test]
    fn send_preserves_per_waiter_out_ptr_even_when_zero() {
        // A guest passing out_ptr == 0 is legal at the ABI level
        // (would typically be a guest bug that writes to guest
        // address 0, but the oracle must faithfully relay what
        // it was given). The old sentinel-merge implementation
        // could not distinguish a legitimate zero from "preserve
        // existing"; the new per-waiter storage handles it
        // transparently.
        let mut t = EventQueueTable::new();
        t.create_with_id(1, 4);
        t.enqueue_waiter(1, tid(0x0100_0001), 0);
        assert_eq!(
            t.send_and_wake_or_enqueue(1, pl(42)),
            EventQueueSend::Woke {
                new_owner: tid(0x0100_0001),
                out_ptr: 0,
                payload: pl(42),
            },
        );
    }

    #[test]
    fn send_when_queue_full_returns_full() {
        let mut t = EventQueueTable::new();
        t.create_with_id(1, 2);
        t.send_and_wake_or_enqueue(1, pl(1));
        t.send_and_wake_or_enqueue(1, pl(2));
        assert_eq!(t.send_and_wake_or_enqueue(1, pl(3)), EventQueueSend::Full);
        assert_eq!(t.lookup(1).unwrap().len(), 2);
    }

    #[test]
    fn send_unknown_id_is_unknown() {
        let mut t = EventQueueTable::new();
        assert_eq!(
            t.send_and_wake_or_enqueue(99, pl(1)),
            EventQueueSend::Unknown
        );
    }

    #[test]
    fn try_receive_batch_drains_up_to_max_in_fifo_order() {
        let mut t = EventQueueTable::new();
        t.create_with_id(1, 10);
        t.send_and_wake_or_enqueue(1, pl(1));
        t.send_and_wake_or_enqueue(1, pl(2));
        t.send_and_wake_or_enqueue(1, pl(3));
        let batch = t.try_receive_batch(1, 2).unwrap();
        assert_eq!(batch, vec![pl(1), pl(2)]);
        // One remaining.
        assert_eq!(t.lookup(1).unwrap().len(), 1);
        let batch2 = t.try_receive_batch(1, 4).unwrap();
        assert_eq!(batch2, vec![pl(3)]);
        // Empty now.
        assert_eq!(t.try_receive_batch(1, 4).unwrap(), vec![]);
    }

    #[test]
    fn try_receive_batch_unknown_is_none() {
        let mut t = EventQueueTable::new();
        assert!(t.try_receive_batch(99, 4).is_none());
    }

    #[test]
    fn state_hash_distinguishes_payload_order() {
        let mut a = EventQueueTable::new();
        let mut b = EventQueueTable::new();
        a.create_with_id(1, 4);
        b.create_with_id(1, 4);
        a.send_and_wake_or_enqueue(1, pl(1));
        a.send_and_wake_or_enqueue(1, pl(2));
        b.send_and_wake_or_enqueue(1, pl(2));
        b.send_and_wake_or_enqueue(1, pl(1));
        assert_ne!(a.state_hash(), b.state_hash());
    }
}
