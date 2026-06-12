//! Lightweight mutex sleep queue.
//!
//! Models the kernel-side primitive only: a `signaled` flag plus a
//! FIFO waiter list. User-space wrappers track owner / recursion /
//! waiter count in the `sys_lwmutex_t` struct and only invoke the
//! kernel on contention, so this mirrors RPCS3's `lv2_lwmutex`
//! (`signaled` + sleep queue) rather than a full mutex.
//!
//! Ids are minted monotonically by [`LwMutexIdAllocator`]; the
//! id space is distinct from the heavy mutex table.

use crate::ppu_thread::PpuThreadId;
use crate::sync_primitives::WaiterList;
use std::collections::BTreeMap;

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
///
/// `signaled` is the binary "wake pending" flag set by an unlock
/// against an empty sleep queue and consumed by the next lock.
/// User-space ownership and recursion tracking live in the
/// guest's `sys_lwmutex_t` struct, not here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LwMutexEntry {
    signaled: bool,
    waiters: WaiterList,
}

impl LwMutexEntry {
    fn new() -> Self {
        // Starts un-signaled: the HLE wrapper only invokes the
        // kernel on contention, so a kernel acquire always means
        // "block until the holder posts a wake".
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

/// Monotonic allocator for lwmutex ids.
///
/// Starts at `1`; last handed-out id is `u32::MAX - 1`. Ids are
/// never recycled.
#[derive(Debug, Clone)]
pub struct LwMutexIdAllocator {
    next: u32,
}

impl Default for LwMutexIdAllocator {
    fn default() -> Self {
        Self::new()
    }
}

impl LwMutexIdAllocator {
    /// Fresh allocator; the first `allocate` returns 1.
    pub fn new() -> Self {
        Self { next: 1 }
    }

    /// Allocate the next id. Returns `None` once exhausted.
    pub fn allocate(&mut self) -> Option<u32> {
        if self.next == u32::MAX {
            return None;
        }
        let id = self.next;
        self.next += 1;
        Some(id)
    }

    /// Fold the allocator's state into `hasher`.
    pub(crate) fn hash_into(&self, hasher: &mut cellgov_mem::Fnv1aHasher) {
        hasher.write(&self.next.to_le_bytes());
    }
}

/// Table of lightweight mutexes.
#[derive(Debug, Clone, Default)]
pub struct LwMutexTable {
    entries: BTreeMap<u32, LwMutexEntry>,
    ids: LwMutexIdAllocator,
    /// Audit C-6a witness: cumulative count of calls to
    /// `acquire_or_enqueue` and `enqueue_waiter`. The catch-all
    /// `debug_assert!`s at lines 234 and 254 of this file guard
    /// internal contains/duplicate-enqueue invariants; silence is
    /// non-vacuous only when the functions actually ran. Not
    /// state-hashed (instrument-only).
    acquires_count: u64,
    /// Audit C-6b witness: cumulative count of
    /// `release_and_wake_next` calls. Pairs with `acquires_count`
    /// on the release side. Not state-hashed.
    releases_count: u64,
}

impl LwMutexTable {
    /// Construct an empty table.
    pub fn new() -> Self {
        Self::default()
    }

    /// Audit C-6a witness: cumulative count of
    /// `acquire_or_enqueue` and `enqueue_waiter` calls. See the
    /// `acquires_count` field doc.
    #[inline]
    pub fn acquires_count(&self) -> u64 {
        self.acquires_count
    }

    /// Audit C-6b witness: cumulative count of
    /// `release_and_wake_next` calls. See the `releases_count`
    /// field doc.
    #[inline]
    pub fn releases_count(&self) -> u64 {
        self.releases_count
    }

    /// Allocate a fresh id and create the entry; `None` if the
    /// id space is exhausted.
    pub fn create(&mut self) -> Option<u32> {
        let id = self.ids.allocate()?;
        self.entries.insert(id, LwMutexEntry::new());
        Some(id)
    }

    /// Remove the entry; `None` if the id was unknown. Ids are
    /// not recycled.
    ///
    /// Caller contract: reject non-empty-waiters before calling
    /// (`debug_assert!` fires on violation). If bypassed in
    /// release, callers **must** drain `entry.waiters()` and wake
    /// each parked thread; skipping this strands them forever.
    pub fn destroy(&mut self, id: u32) -> Option<LwMutexEntry> {
        let entry = self.entries.remove(&id)?;
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
        self.entries.get(&id)
    }

    /// Number of tracked mutexes.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the table has no entries.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Iterate ids in `BTreeMap` order.
    pub fn iter_ids(&self) -> impl Iterator<Item = u32> + '_ {
        self.entries.keys().copied()
    }

    /// Try to consume a pending signal without enqueueing.
    ///
    /// Returns `Acquired` (signal consumed, caller proceeds) or
    /// `Contended` (no signal, no state change). Owner / recursion
    /// checks happen in the user-space wrapper before this entry
    /// point fires.
    pub fn try_acquire(&mut self, id: u32, _caller: PpuThreadId) -> Option<LwMutexAcquire> {
        let entry = self.entries.get_mut(&id)?;
        if entry.signaled {
            entry.signaled = false;
            Some(LwMutexAcquire::Acquired)
        } else {
            Some(LwMutexAcquire::Contended)
        }
    }

    /// Atomic acquire-or-park.
    ///
    /// If the entry is signaled, the caller consumes the signal and
    /// proceeds (`Acquired`). Otherwise the caller is appended to
    /// the FIFO sleep queue and must block (`Enqueued`). A caller
    /// already on the sleep queue indicates a dispatch-layer bug
    /// and returns `WouldDeadlock`.
    ///
    /// O(n) scan over the waiter list on the already-parked check.
    pub fn acquire_or_enqueue(&mut self, id: u32, caller: PpuThreadId) -> LwMutexAcquireOrEnqueue {
        self.acquires_count = self.acquires_count.wrapping_add(1);
        let Some(entry) = self.entries.get_mut(&id) else {
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
        let entry = self
            .entries
            .get_mut(&id)
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

    /// Release on behalf of `caller`. Wakes the head of the sleep
    /// queue if any waiter is parked (`Transferred`), otherwise sets
    /// the signal so the next lock-call passes without blocking
    /// (`Signaled`). The kernel does not validate `_caller`; the
    /// user-space wrapper verifies the owner before invoking unlock.
    pub fn release_and_wake_next(&mut self, id: u32, _caller: PpuThreadId) -> LwMutexRelease {
        self.releases_count = self.releases_count.wrapping_add(1);
        let Some(entry) = self.entries.get_mut(&id) else {
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

    /// FNV-1a digest of the table's state, including the
    /// id-allocator cursor.
    pub fn state_hash(&self) -> u64 {
        let mut hasher = cellgov_mem::Fnv1aHasher::new();
        self.ids.hash_into(&mut hasher);
        hasher.write(&(self.entries.len() as u64).to_le_bytes());
        for (id, entry) in &self.entries {
            hasher.write(&id.to_le_bytes());
            hasher.write(&[entry.signaled as u8]);
            hasher.write(&(entry.waiters.len() as u64).to_le_bytes());
            for waiter in entry.waiters.iter() {
                hasher.write(&waiter.raw().to_le_bytes());
            }
        }
        hasher.finish()
    }
}

#[cfg(test)]
#[path = "tests/lwmutex_tests.rs"]
mod tests;
