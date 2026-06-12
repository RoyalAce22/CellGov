//! Event queue table.
//!
//! Storage invariant: an entry never holds both buffered
//! payloads and parked waiters. `send_and_wake_or_enqueue` hands
//! off directly when a waiter exists; `enqueue_waiter`
//! `debug_assert!`s the payload list is empty before parking.

use crate::ppu_thread::PpuThreadId;
use std::collections::{BTreeMap, VecDeque};

/// One queued event. Matches the `sys_event_t` layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EventPayload {
    /// Source id.
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
    /// Payload popped.
    Delivered(EventPayload),
    /// Queue empty.
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
    /// Buffered payload count already equals `size`.
    Full,
    /// Unknown id.
    Unknown,
}

/// Failure modes of [`EventQueueTable::enqueue_waiter`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum EventQueueEnqueueError {
    /// No event queue with this id.
    #[error("event_queue enqueue: unknown id")]
    UnknownId,
    /// Thread is already parked on this queue; dispatch-layer
    /// bug (fires `debug_assert!`).
    #[error("event_queue enqueue: duplicate waiter")]
    DuplicateWaiter,
}

/// A parked `sys_event_queue_receive` caller.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EventQueueWaiter {
    /// Parked thread.
    pub thread: PpuThreadId,
    /// Guest address of the `sys_event_t` output buffer.
    pub out_ptr: u32,
}

/// A single event queue.
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

    /// Whether the payload buffer is empty. Does not consult the
    /// waiter list; use [`Self::is_inert`] at destruction sites.
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

    /// Safe-to-destroy predicate: no payloads AND no waiters.
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

    /// Remove the entry; `None` if the id was unknown.
    ///
    /// Caller contract: reject non-empty-waiters before calling
    /// (`debug_assert!` fires on violation). If bypassed in
    /// release, callers **must** drain `entry.waiters()` and wake
    /// each parked thread; skipping this strands them forever.
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

    /// Whether the table has no entries.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Try to pop a payload; `None` if `id` is unknown.
    pub fn try_receive(&mut self, id: u32) -> Option<EventQueueReceive> {
        let entry = self.entries.get_mut(&id)?;
        // Mutual-exclusion invariant: the queue holds buffered
        // payloads OR parked waiters, never both. Without this
        // guard `try_receive` could pop a head payload while
        // leaving a parked waiter on a drained queue.
        if !(entry.waiters.is_empty() || entry.payloads.is_empty()) {
            #[allow(
                clippy::print_stderr,
                reason = "one-shot release-build diagnostic for a host invariant break that is not guest-reachable under normal operation"
            )]
            {
                eprintln!(
                    "lv2 host invariant break at sync_primitives.event_queue.mutual_exclusion: \
                     event queue {:#x} has {} buffered payload(s) AND {} parked waiter(s); \
                     waiters may be stranded on a drained queue.",
                    id,
                    entry.payloads.len(),
                    entry.waiters.len(),
                );
            }
        }
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

    /// Drain up to `max` payloads in arrival order; `None` if
    /// `id` is unknown.
    pub fn try_receive_batch(&mut self, id: u32, max: usize) -> Option<Vec<EventPayload>> {
        let entry = self.entries.get_mut(&id)?;
        // Same mutual-exclusion invariant as `try_receive`.
        if !(entry.waiters.is_empty() || entry.payloads.is_empty()) {
            #[allow(
                clippy::print_stderr,
                reason = "one-shot release-build diagnostic for a host invariant break that is not guest-reachable under normal operation"
            )]
            {
                eprintln!(
                    "lv2 host invariant break at sync_primitives.event_queue.mutual_exclusion: \
                     event queue {:#x} has {} buffered payload(s) AND {} parked waiter(s); \
                     waiters may be stranded on a drained queue.",
                    id,
                    entry.payloads.len(),
                    entry.waiters.len(),
                );
            }
        }
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

    /// Enqueue a receive-waiter.
    ///
    /// Precondition: caller must have seen
    /// [`EventQueueReceive::Empty`] from [`Self::try_receive`].
    /// Parking with payloads buffered violates the storage
    /// invariant; `debug_assert!` catches it.
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

    /// Send a payload. Hands off to the head waiter, or buffers,
    /// or returns `Full` at `size`.
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

    /// FNV-1a digest of the table's state.
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
#[path = "tests/event_queue_tests.rs"]
mod tests;
