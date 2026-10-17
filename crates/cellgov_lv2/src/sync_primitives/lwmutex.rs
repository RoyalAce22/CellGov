//! Lightweight mutex sleep queue.
//!
//! Models the kernel-side primitive only: a `signaled` flag plus a
//! FIFO waiter list. The user-space wrapper owns the rest. liblv2's
//! `sys_lwmutex_lock` compare-and-swaps the owner word of the
//! caller's `sys_lwmutex_t` and bumps the in-struct recursion counter
//! on an owner re-lock. It reaches the kernel (syscall 97) only after
//! the contended spin fails.
//!
//! The host mints each id from its shared kernel-id table, as for
//! every other sync object.

use crate::ppu_thread::PpuThreadId;
use crate::sync_primitives::WaiterList;
use cellgov_mem::lanes::{source, LaneMap, LaneValue, ObjectLanes};

/// Outcome of a `try_acquire` call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LwMutexAcquire {
    /// The signal was consumed; caller proceeds without blocking.
    Acquired,
    /// No signal pending.
    Contended,
}

/// Outcome of an `acquire_or_enqueue` call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LwMutexAcquireOrEnqueue {
    /// Signal consumed; caller proceeds without blocking.
    Acquired,
    /// Caller was appended to the waiter list and must block.
    Enqueued,
    /// Caller is already parked on this mutex; dispatch-layer bug.
    WouldDeadlock,
    /// Unknown id.
    Unknown,
}

/// Outcome of a `release_and_wake_next` call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LwMutexRelease {
    /// Sleep queue was empty; the signal was set so the next lock
    /// will pass without blocking.
    Signaled,
    /// Ownership transferred to `new_owner`; caller must wake it.
    Transferred {
        /// Thread that was at the head of the sleep queue.
        new_owner: PpuThreadId,
    },
    /// Unknown id.
    Unknown,
}

/// Failure modes of [`LwMutexTable::enqueue_waiter`].
///
/// All non-`UnknownId` variants indicate dispatch-layer bugs and
/// fire `debug_assert!`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum LwMutexEnqueueError {
    /// No lwmutex with this id.
    #[error("lwmutex enqueue: unknown id")]
    UnknownId,
    /// Thread is already on the waiter list.
    #[error("lwmutex enqueue: duplicate waiter")]
    DuplicateWaiter,
}

/// A single lightweight mutex.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LwMutexEntry {
    signaled: bool,
    waiters: WaiterList,
}

impl LwMutexEntry {
    fn new() -> Self {
        Self {
            signaled: false,
            waiters: WaiterList::new(),
        }
    }

    /// Whether a wake is pending for the next lock-call.
    pub fn signaled(&self) -> bool {
        self.signaled
    }

    /// Read-only view of the waiter list.
    pub fn waiters(&self) -> &WaiterList {
        &self.waiters
    }
}

impl LaneValue for LwMutexEntry {
    fn lanes(&self, lanes: &mut ObjectLanes) {
        lanes.lane(1, 0, u64::from(self.signaled));
        self.waiters.push_lanes(lanes, 2);
    }
}

/// Table of lightweight mutexes.
#[derive(Debug, Clone)]
pub struct LwMutexTable {
    entries: LaneMap<u32, LwMutexEntry>,
    /// See [`Self::acquires_count`].
    acquires_count: u64,
    /// See [`Self::releases_count`].
    releases_count: u64,
}

impl Default for LwMutexTable {
    fn default() -> Self {
        Self {
            entries: LaneMap::new(source::LWMUTEX, u64::from),
            acquires_count: 0,
            releases_count: 0,
        }
    }
}

impl LwMutexTable {
    /// Construct an empty table.
    pub fn new() -> Self {
        Self::default()
    }

    /// Cumulative `acquire_or_enqueue` + `enqueue_waiter` calls.
    /// Not part of [`Self::sync_partial`].
    #[inline]
    pub fn acquires_count(&self) -> u64 {
        self.acquires_count
    }

    /// Cumulative `release_and_wake_next` calls; the release-side
    /// counterpart of [`Self::acquires_count`]. Not part of
    /// [`Self::sync_partial`].
    #[inline]
    pub fn releases_count(&self) -> u64 {
        self.releases_count
    }

    /// Create the entry for the minted `id`; `false`, with the table
    /// unchanged, when an entry already holds it.
    pub fn create_with_id(&mut self, id: u32) -> bool {
        if self.entries.contains_key(id) {
            return false;
        }
        self.entries.insert(id, LwMutexEntry::new());
        true
    }

    /// Remove the entry; `None` if the id was unknown.
    ///
    /// Caller contract: reject non-empty-waiters before calling
    /// (`debug_assert!` fires on violation). If bypassed in
    /// release, callers **must** drain `entry.waiters()` and wake
    /// each parked thread; skipping this strands them forever.
    pub fn destroy(&mut self, id: u32) -> Option<LwMutexEntry> {
        let entry = self.entries.remove(id)?;
        debug_assert!(
            entry.waiters.is_empty(),
            "lwmutex {:#x} destroyed with {} parked waiter(s)",
            id,
            entry.waiters.len(),
        );
        Some(entry)
    }

    /// Read-only lookup.
    pub fn lookup(&self, id: u32) -> Option<&LwMutexEntry> {
        self.entries.get(id)
    }

    /// Number of tracked mutexes.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the table has no entries.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Iterate ids in ascending order.
    pub fn iter_ids(&self) -> impl Iterator<Item = u32> + '_ {
        self.entries.keys()
    }

    /// Try to consume a pending signal without enqueueing.
    ///
    /// Owner / recursion checks happen in the user-space wrapper
    /// before this entry point fires.
    pub fn try_acquire(&mut self, id: u32, _caller: PpuThreadId) -> Option<LwMutexAcquire> {
        let mut entry = self.entries.get_mut(id)?;
        if entry.signaled {
            entry.signaled = false;
            Some(LwMutexAcquire::Acquired)
        } else {
            Some(LwMutexAcquire::Contended)
        }
    }

    /// Atomic acquire-or-park.
    ///
    /// O(n) scan over the waiter list on the already-parked check.
    pub fn acquire_or_enqueue(&mut self, id: u32, caller: PpuThreadId) -> LwMutexAcquireOrEnqueue {
        self.acquires_count = self.acquires_count.wrapping_add(1);
        let Some(mut entry) = self.entries.get_mut(id) else {
            return LwMutexAcquireOrEnqueue::Unknown;
        };
        if entry.signaled {
            entry.signaled = false;
            return LwMutexAcquireOrEnqueue::Acquired;
        }
        if entry.waiters.contains(caller) {
            return LwMutexAcquireOrEnqueue::WouldDeadlock;
        }
        if entry.waiters.enqueue(caller).is_err() {
            debug_assert!(
                false,
                "contains guard broken for lwmutex {id:#x} caller {caller:?}"
            );
        }
        LwMutexAcquireOrEnqueue::Enqueued
    }

    /// Low-level enqueue. Prefer [`Self::acquire_or_enqueue`]
    /// for blocking lock paths.
    pub fn enqueue_waiter(
        &mut self,
        id: u32,
        waiter: PpuThreadId,
    ) -> Result<(), LwMutexEnqueueError> {
        self.acquires_count = self.acquires_count.wrapping_add(1);
        let mut entry = self
            .entries
            .get_mut(id)
            .ok_or(LwMutexEnqueueError::UnknownId)?;
        if entry.waiters.enqueue(waiter).is_err() {
            debug_assert!(
                false,
                "duplicate enqueue of {:?} on lwmutex {:#x}",
                waiter, id,
            );
            return Err(LwMutexEnqueueError::DuplicateWaiter);
        }
        Ok(())
    }

    /// Remove every waiter in `threads` from every lwmutex, preserving
    /// the order of survivors; returns `(id, thread)` pairs in table
    /// order. Process-exit purge; the signaled flag is untouched.
    #[must_use = "the purged pairs are the only witness that these wakes were cancelled"]
    pub fn purge_waiters_of(
        &mut self,
        threads: &std::collections::BTreeSet<PpuThreadId>,
    ) -> Vec<(u32, PpuThreadId)> {
        let mut removed = Vec::new();
        self.entries.for_each_mut(|id, entry| {
            for thread in entry.waiters.remove_set(threads) {
                removed.push((id, thread));
            }
        });
        removed
    }

    /// Remove `waiter` from the sleep queue without granting the
    /// lock; `false` if the id is unknown or the thread is not
    /// parked. Timeout-expiry cancel; order-preserving for the rest.
    pub fn remove_waiter(&mut self, id: u32, waiter: PpuThreadId) -> bool {
        let Some(mut entry) = self.entries.get_mut(id) else {
            return false;
        };
        entry.waiters.remove(waiter)
    }

    /// Release: wake the sleep-queue head or set the signal.
    ///
    /// The kernel does not validate `_caller`; the user-space
    /// wrapper verifies the owner before invoking unlock.
    pub fn release_and_wake_next(&mut self, id: u32, _caller: PpuThreadId) -> LwMutexRelease {
        self.releases_count = self.releases_count.wrapping_add(1);
        let Some(mut entry) = self.entries.get_mut(id) else {
            return LwMutexRelease::Unknown;
        };
        match entry.waiters.dequeue_one() {
            Some(new_owner) => LwMutexRelease::Transferred { new_owner },
            None => {
                entry.signaled = true;
                LwMutexRelease::Signaled
            }
        }
    }

    /// The table's partial of the sync-state sum.
    pub fn sync_partial(&self) -> u128 {
        self.entries.partial()
    }

    /// [`Self::sync_partial`] computed from every entry.
    pub fn sync_partial_from_scratch(&self) -> u128 {
        self.entries.partial_from_scratch()
    }
}

#[cfg(test)]
#[path = "tests/lwmutex_tests.rs"]
mod tests;
