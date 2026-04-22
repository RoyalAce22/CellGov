//! Event queue table. Owns state for `sys_event_queue_create` /
//! `_destroy` / `_receive` / `_send` / `_tryreceive`.
//!
//! Storage invariant: an entry never holds both buffered
//! payloads and parked waiters. `send_and_wake_or_enqueue` hands
//! off directly when a waiter exists; `enqueue_waiter`
//! `debug_assert!`s the payload list is empty before parking.
//! The invariant is what makes [`EventQueueTable::try_receive`]
//! FIFO-correct -- pop ordering can never jump ahead of a
//! parked waiter because parked waiters cannot coexist with
//! buffered payloads.

use crate::ppu_thread::PpuThreadId;
use std::collections::{BTreeMap, VecDeque};

/// One queued event. Matches the PS3 `sys_event_t` layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EventPayload {
    /// Source id (typically a sending event-port id or a
    /// user-defined tag).
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
    /// A payload was popped and should be delivered.
    Delivered(EventPayload),
    /// Queue empty; caller parks (for `_receive`) or returns
    /// `ENOENT` (for `_tryreceive`).
    Empty,
}

/// Outcome of a `send_and_wake_or_enqueue` call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventQueueSend {
    /// Payload handed directly to a parked waiter.
    Woke {
        /// Thread that received the payload.
        new_owner: PpuThreadId,
        /// Out pointer the waiter supplied at wait time.
        out_ptr: u32,
        /// Payload to deliver on wake.
        payload: EventPayload,
    },
    /// No waiter; payload was buffered.
    Enqueued,
    /// Buffered payload count already equals `size`; caller
    /// returns `EBUSY`.
    Full,
    /// Unknown id.
    Unknown,
}

/// Failure modes of [`EventQueueTable::enqueue_waiter`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventQueueEnqueueError {
    /// No event queue with this id.
    UnknownId,
    /// Thread is already parked on this queue. A single PPU
    /// thread cannot be in two `sys_event_queue_receive`
    /// syscalls at once; dispatch-layer bug. `debug_assert!`
    /// fires.
    DuplicateWaiter,
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
/// The waiter list carries per-waiter `out_ptr`s rather than
/// using the shared `WaiterList`, so the send side can build a
/// complete `PendingResponse::EventQueueReceive` at wake time
/// without a separate continuation lookup.
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

    /// Whether the payload buffer is empty. Does NOT consult
    /// the waiter list -- a queue with parked receivers still
    /// returns `true` here. Use [`Self::is_inert`] at
    /// destruction sites; using `is_empty` there would strand
    /// parked threads.
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

    /// `true` iff no payloads buffered AND no waiters parked --
    /// the "safe to destroy" predicate.
    pub fn is_inert(&self) -> bool {
        self.payloads.is_empty() && self.waiters.is_empty()
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

    /// Insert a fresh entry. Returns `false` if `id` is already
    /// present or `size == 0`.
    pub fn create_with_id(&mut self, id: u32, size: u32) -> bool {
        if self.entries.contains_key(&id) || size == 0 {
            return false;
        }
        self.entries.insert(id, EventQueueEntry::new(size));
        true
    }

    /// Destroy an event queue and return the removed entry, or
    /// `None` if the id was unknown.
    ///
    /// Caller contract: reject non-empty-waiters before calling.
    /// `debug_assert!` fires on violation. If bypassed in
    /// release, the returned entry carries the waiter list and
    /// callers **must** drain and wake each parked thread;
    /// skipping this strands them forever.
    pub fn destroy(&mut self, id: u32) -> Option<EventQueueEntry> {
        let entry = self.entries.remove(&id)?;
        debug_assert!(
            entry.waiters.is_empty(),
            "event queue {:#x} destroyed with {} parked waiter(s)",
            id,
            entry.waiters.len(),
        );
        Some(entry)
    }

    /// Read-only lookup.
    pub fn lookup(&self, id: u32) -> Option<&EventQueueEntry> {
        self.entries.get(&id)
    }

    /// Number of tracked queues.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the table has no entries. Distinct from
    /// [`EventQueueEntry::is_empty`], which reports whether a
    /// single queue has no buffered payloads.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Try to pop a payload. `None` if `id` is unknown.
    ///
    /// A `debug_assert!` checks the storage invariant
    /// (payloads-xor-waiters); violation would allow this call
    /// to pop ahead of a parked waiter.
    pub fn try_receive(&mut self, id: u32) -> Option<EventQueueReceive> {
        let entry = self.entries.get_mut(&id)?;
        debug_assert!(
            entry.waiters.is_empty() || entry.payloads.is_empty(),
            "event queue {:#x} has {} buffered payload(s) AND {} parked waiter(s)",
            id,
            entry.payloads.len(),
            entry.waiters.len(),
        );
        match entry.payloads.pop_front() {
            Some(p) => Some(EventQueueReceive::Delivered(p)),
            None => Some(EventQueueReceive::Empty),
        }
    }

    /// Non-blocking batch pop: drain up to `max` payloads in
    /// arrival order. `None` if `id` is unknown; empty `Vec` if
    /// no payloads buffered. Shares [`Self::try_receive`]'s
    /// invariant assertion.
    pub fn try_receive_batch(&mut self, id: u32, max: usize) -> Option<Vec<EventPayload>> {
        let entry = self.entries.get_mut(&id)?;
        debug_assert!(
            entry.waiters.is_empty() || entry.payloads.is_empty(),
            "event queue {:#x} has {} buffered payload(s) AND {} parked waiter(s)",
            id,
            entry.payloads.len(),
            entry.waiters.len(),
        );
        let n = entry.payloads.len().min(max);
        let mut out = Vec::with_capacity(n);
        for _ in 0..n {
            out.push(entry.payloads.pop_front().expect("pop_front within len"));
        }
        Some(out)
    }

    /// Enqueue a receive-waiter. See [`EventQueueEnqueueError`].
    ///
    /// Precondition: caller must have seen
    /// [`EventQueueReceive::Empty`] from [`Self::try_receive`].
    /// Parking while payloads are buffered violates the storage
    /// invariant (send's fast-path would hand directly to the
    /// new waiter, stranding the buffered payloads); a
    /// `debug_assert!` catches it.
    pub fn enqueue_waiter(
        &mut self,
        id: u32,
        thread: PpuThreadId,
        out_ptr: u32,
    ) -> Result<(), EventQueueEnqueueError> {
        let entry = self
            .entries
            .get_mut(&id)
            .ok_or(EventQueueEnqueueError::UnknownId)?;
        debug_assert!(
            entry.payloads.is_empty(),
            "thread {:?} enqueued on event queue {:#x} with {} buffered payload(s)",
            thread,
            id,
            entry.payloads.len(),
        );
        if entry.waiters.iter().any(|w| w.thread == thread) {
            debug_assert!(
                false,
                "duplicate enqueue of {:?} on event queue {:#x}",
                thread, id,
            );
            return Err(EventQueueEnqueueError::DuplicateWaiter);
        }
        entry
            .waiters
            .push_back(EventQueueWaiter { thread, out_ptr });
        Ok(())
    }

    /// Send a payload. Hands off to the head waiter if one is
    /// parked (returning `Woke` with the waiter's recorded
    /// `out_ptr`); otherwise buffers, or returns `Full` at
    /// `size`.
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
                if entry.payloads.len() >= entry.size as usize {
                    EventQueueSend::Full
                } else {
                    entry.payloads.push_back(payload);
                    EventQueueSend::Enqueued
                }
            }
        }
    }

    /// FNV-1a digest for state-hash folding. Walks entries in
    /// ascending-id order; folds size, payload FIFO, and waiter
    /// FIFO (including `out_ptr`s).
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
        let mut t = EventQueueTable::new();
        t.create_with_id(1, 4);
        t.enqueue_waiter(1, tid(0x0100_0001), 0x2000).unwrap();
        assert_eq!(
            t.send_and_wake_or_enqueue(1, pl(42)),
            EventQueueSend::Woke {
                new_owner: tid(0x0100_0001),
                out_ptr: 0x2000,
                payload: pl(42),
            },
        );
        assert!(t.lookup(1).unwrap().is_empty());
        assert!(t.lookup(1).unwrap().waiters().is_empty());
    }

    #[test]
    fn send_wakes_waiters_in_fifo_order() {
        let mut t = EventQueueTable::new();
        t.create_with_id(1, 4);
        t.enqueue_waiter(1, tid(0x0100_0001), 0x2000).unwrap();
        t.enqueue_waiter(1, tid(0x0100_0002), 0x2020).unwrap();
        t.enqueue_waiter(1, tid(0x0100_0003), 0x2040).unwrap();
        assert_eq!(
            t.send_and_wake_or_enqueue(1, pl(10)),
            EventQueueSend::Woke {
                new_owner: tid(0x0100_0001),
                out_ptr: 0x2000,
                payload: pl(10),
            },
        );
        assert_eq!(
            t.send_and_wake_or_enqueue(1, pl(20)),
            EventQueueSend::Woke {
                new_owner: tid(0x0100_0002),
                out_ptr: 0x2020,
                payload: pl(20),
            },
        );
        assert_eq!(
            t.send_and_wake_or_enqueue(1, pl(30)),
            EventQueueSend::Woke {
                new_owner: tid(0x0100_0003),
                out_ptr: 0x2040,
                payload: pl(30),
            },
        );
        assert_eq!(
            t.send_and_wake_or_enqueue(1, pl(40)),
            EventQueueSend::Enqueued
        );
    }

    #[test]
    fn send_preserves_per_waiter_out_ptr_even_when_zero() {
        let mut t = EventQueueTable::new();
        t.create_with_id(1, 4);
        t.enqueue_waiter(1, tid(0x0100_0001), 0).unwrap();
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
        assert_eq!(t.lookup(1).unwrap().len(), 1);
        let batch2 = t.try_receive_batch(1, 4).unwrap();
        assert_eq!(batch2, vec![pl(3)]);
        assert_eq!(t.try_receive_batch(1, 4).unwrap(), vec![]);
    }

    #[test]
    fn try_receive_batch_unknown_is_none() {
        let mut t = EventQueueTable::new();
        assert!(t.try_receive_batch(99, 4).is_none());
    }

    #[test]
    fn enqueue_waiter_unknown_id_returns_err() {
        let mut t = EventQueueTable::new();
        assert_eq!(
            t.enqueue_waiter(99, tid(0x0100_0001), 0x2000),
            Err(EventQueueEnqueueError::UnknownId),
        );
    }

    #[test]
    fn is_inert_reflects_both_axes() {
        let mut t = EventQueueTable::new();
        t.create_with_id(1, 4);
        assert!(t.lookup(1).unwrap().is_inert());
        t.send_and_wake_or_enqueue(1, pl(1));
        assert!(!t.lookup(1).unwrap().is_empty());
        assert!(!t.lookup(1).unwrap().is_inert());
        let _ = t.try_receive(1);
        assert!(t.lookup(1).unwrap().is_inert());
        t.enqueue_waiter(1, tid(0x0100_0001), 0x2000).unwrap();
        // Payloads empty yet queue not inert (parked waiter).
        assert!(t.lookup(1).unwrap().is_empty());
        assert!(!t.lookup(1).unwrap().is_inert());
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

    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "duplicate enqueue")]
    fn duplicate_enqueue_panics_in_debug() {
        let mut t = EventQueueTable::new();
        t.create_with_id(1, 4);
        t.enqueue_waiter(1, tid(0x0100_0001), 0x2000).unwrap();
        let _ = t.enqueue_waiter(1, tid(0x0100_0001), 0x2000);
    }

    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "payload(s)")]
    fn enqueue_when_payloads_present_panics_in_debug() {
        let mut t = EventQueueTable::new();
        t.create_with_id(1, 4);
        t.send_and_wake_or_enqueue(1, pl(1));
        let _ = t.enqueue_waiter(1, tid(0x0100_0001), 0x2000);
    }

    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "destroyed with")]
    fn destroy_with_parked_waiters_panics_in_debug() {
        let mut t = EventQueueTable::new();
        t.create_with_id(1, 4);
        t.enqueue_waiter(1, tid(0x0100_0001), 0x2000).unwrap();
        let _ = t.destroy(1);
    }

    #[test]
    #[cfg(not(debug_assertions))]
    fn duplicate_enqueue_returns_err_in_release() {
        let mut t = EventQueueTable::new();
        t.create_with_id(1, 4);
        t.enqueue_waiter(1, tid(0x0100_0001), 0x2000).unwrap();
        assert_eq!(
            t.enqueue_waiter(1, tid(0x0100_0001), 0x2000),
            Err(EventQueueEnqueueError::DuplicateWaiter),
        );
        assert_eq!(t.lookup(1).unwrap().waiters().len(), 1);
    }
}
