//! Lightweight mutex sleep queue.
//!
//! Models the kernel-side primitive only: a `signaled` flag plus a
//! FIFO waiter list. PSL1GHT (and the PS3 SDK static-libc) tracks
//! owner / recursion / waiter count in the user-space `sys_lwmutex_t`
//! struct and only invokes the kernel for actual contention; the
//! kernel-side object therefore mirrors RPCS3's `lv2_lwmutex`
//! (`signaled` + sleep queue), not a full mutex with ownership.
//!
//! Initial state of a fresh entry is `signaled = true`: the first
//! lock-call to land in the kernel consumes the signal and returns
//! without blocking, matching real PS3 behaviour where a freshly
//! created lwmutex is takeable.
//!
//! Ids are minted monotonically by [`LwMutexIdAllocator`]. The
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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LwMutexEnqueueError {
    /// No lwmutex with this id.
    UnknownId,
    /// Thread is already on the waiter list.
    DuplicateWaiter,
}

/// A single lightweight mutex.
///
/// `signaled` is the binary "wake pending" flag set by an unlock
/// against an empty sleep queue and consumed by the next lock.
/// PSL1GHT-side ownership and recursion tracking live in the
/// guest's `sys_lwmutex_t` struct, not here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LwMutexEntry {
    signaled: bool,
    waiters: WaiterList,
}

impl LwMutexEntry {
    fn new() -> Self {
        // A freshly created lwmutex starts un-signaled. The HLE
        // wrapper for `sys_lwmutex_lock` only invokes the kernel
        // on actual contention (its user-space CAS already covers
        // the uncontended case), so a kernel-side `acquire` always
        // means "block until the holder posts a wake". Starting
        // `signaled = true` would silently break that invariant by
        // letting the very first contender skip the queue while
        // the holder still owns the user-space struct.
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
}

impl LwMutexTable {
    /// Construct an empty table.
    pub fn new() -> Self {
        Self::default()
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
    /// `Contended` (no signal, no state change). PSL1GHT-style
    /// owner / recursion checks happen in the user-space wrapper
    /// before this entry point fires.
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

    /// Release on behalf of `caller`. Recursive: a release that
    /// Release. Wakes the head of the sleep queue if any waiter is
    /// parked (`Transferred`), otherwise sets the signal so the next
    /// lock-call passes without blocking (`Signaled`). The kernel
    /// does not validate `_caller`; PSL1GHT verifies the owner in
    /// user space before invoking unlock.
    pub fn release_and_wake_next(&mut self, id: u32, _caller: PpuThreadId) -> LwMutexRelease {
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
mod tests {
    use super::*;

    fn tid(raw: u64) -> PpuThreadId {
        PpuThreadId::new(raw)
    }

    #[test]
    fn fresh_table_is_empty() {
        let t = LwMutexTable::new();
        assert!(t.is_empty());
        assert_eq!(t.len(), 0);
        assert!(t.lookup(1).is_none());
    }

    #[test]
    fn id_allocator_is_monotonic_and_starts_at_one() {
        let mut a = LwMutexIdAllocator::new();
        assert_eq!(a.allocate(), Some(1));
        assert_eq!(a.allocate(), Some(2));
        assert_eq!(a.allocate(), Some(3));
    }

    #[test]
    fn id_allocator_exhaustion() {
        let mut a = LwMutexIdAllocator { next: u32::MAX };
        assert_eq!(a.allocate(), None);
        assert_eq!(a.allocate(), None);
    }

    #[test]
    fn id_allocator_last_handed_out_is_u32_max_minus_one() {
        let mut a = LwMutexIdAllocator { next: u32::MAX - 1 };
        assert_eq!(a.allocate(), Some(u32::MAX - 1));
        assert_eq!(a.allocate(), None);
    }

    #[test]
    fn create_returns_monotonic_ids() {
        let mut t = LwMutexTable::new();
        let id1 = t.create().unwrap();
        let id2 = t.create().unwrap();
        let id3 = t.create().unwrap();
        assert!(id1 < id2 && id2 < id3);
        assert_eq!(t.len(), 3);
    }

    #[test]
    fn destroy_removes_entry_and_does_not_recycle_id() {
        let mut t = LwMutexTable::new();
        let id1 = t.create().unwrap();
        assert!(t.destroy(id1).is_some());
        assert!(t.lookup(id1).is_none());
        let id2 = t.create().unwrap();
        assert!(id2 > id1);
    }

    #[test]
    fn fresh_entry_is_unsignaled() {
        // A freshly created entry starts un-signaled. Locks reaching
        // the kernel always park; only an unlock against an empty
        // queue sets the signal so the next contender can pass.
        let mut t = LwMutexTable::new();
        let id = t.create().unwrap();
        assert!(!t.lookup(id).unwrap().signaled());
    }

    #[test]
    fn try_acquire_unsignaled_is_contended() {
        let mut t = LwMutexTable::new();
        let id = t.create().unwrap();
        let caller = tid(0x0100_0001);
        assert_eq!(t.try_acquire(id, caller), Some(LwMutexAcquire::Contended));
        assert!(!t.lookup(id).unwrap().signaled());
    }

    #[test]
    fn try_acquire_consumes_signal_set_by_unlock() {
        // After an unlock-with-no-waiters sets the signal, the next
        // try_acquire consumes it.
        let mut t = LwMutexTable::new();
        let id = t.create().unwrap();
        let a = tid(0x0100_0001);
        t.release_and_wake_next(id, a);
        assert!(t.lookup(id).unwrap().signaled());
        assert_eq!(t.try_acquire(id, a), Some(LwMutexAcquire::Acquired));
        assert!(!t.lookup(id).unwrap().signaled());
    }

    #[test]
    fn try_acquire_unknown_id_is_none() {
        let mut t = LwMutexTable::new();
        assert!(t.try_acquire(99, tid(0x0100_0001)).is_none());
    }

    #[test]
    fn acquire_or_enqueue_unsignaled_parks() {
        let mut t = LwMutexTable::new();
        let id = t.create().unwrap();
        let a = tid(0x0100_0001);
        assert_eq!(
            t.acquire_or_enqueue(id, a),
            LwMutexAcquireOrEnqueue::Enqueued,
        );
        assert_eq!(t.lookup(id).unwrap().waiters().len(), 1);
    }

    #[test]
    fn acquire_or_enqueue_consumes_signal_when_pending() {
        let mut t = LwMutexTable::new();
        let id = t.create().unwrap();
        // Set the signal via an unlock against an empty queue.
        t.release_and_wake_next(id, tid(0x0100_0001));
        assert_eq!(
            t.acquire_or_enqueue(id, tid(0x0100_0002)),
            LwMutexAcquireOrEnqueue::Acquired,
        );
        assert!(!t.lookup(id).unwrap().signaled());
    }

    #[test]
    fn acquire_or_enqueue_already_parked_is_would_deadlock() {
        let mut t = LwMutexTable::new();
        let id = t.create().unwrap();
        let waiter = tid(0x0100_0002);
        t.acquire_or_enqueue(id, waiter);
        assert_eq!(
            t.acquire_or_enqueue(id, waiter),
            LwMutexAcquireOrEnqueue::WouldDeadlock,
        );
        assert_eq!(t.lookup(id).unwrap().waiters().len(), 1);
    }

    #[test]
    fn acquire_or_enqueue_unknown_id_is_unknown() {
        let mut t = LwMutexTable::new();
        assert_eq!(
            t.acquire_or_enqueue(99, tid(0x0100_0001)),
            LwMutexAcquireOrEnqueue::Unknown,
        );
    }

    #[test]
    fn enqueue_waiter_preserves_fifo_order() {
        let mut t = LwMutexTable::new();
        let id = t.create().unwrap();
        // Consume the initial signal first.
        t.try_acquire(id, tid(0x0100_0001));
        t.enqueue_waiter(id, tid(0x0100_0002)).unwrap();
        t.enqueue_waiter(id, tid(0x0100_0003)).unwrap();
        t.enqueue_waiter(id, tid(0x0100_0004)).unwrap();
        let seen: Vec<_> = t.lookup(id).unwrap().waiters().iter().collect();
        assert_eq!(
            seen,
            vec![tid(0x0100_0002), tid(0x0100_0003), tid(0x0100_0004)],
        );
    }

    #[test]
    fn enqueue_waiter_unknown_id_returns_err() {
        let mut t = LwMutexTable::new();
        assert_eq!(
            t.enqueue_waiter(99, tid(0x0100_0001)),
            Err(LwMutexEnqueueError::UnknownId),
        );
    }

    #[test]
    fn release_without_waiters_signals() {
        let mut t = LwMutexTable::new();
        let id = t.create().unwrap();
        let a = tid(0x0100_0001);
        t.try_acquire(id, a);
        assert_eq!(t.release_and_wake_next(id, a), LwMutexRelease::Signaled);
        assert!(t.lookup(id).unwrap().signaled());
    }

    #[test]
    fn release_with_waiters_transfers_to_head() {
        let mut t = LwMutexTable::new();
        let id = t.create().unwrap();
        let owner = tid(0x0100_0001);
        let w1 = tid(0x0100_0002);
        let w2 = tid(0x0100_0003);
        t.try_acquire(id, owner);
        t.enqueue_waiter(id, w1).unwrap();
        t.enqueue_waiter(id, w2).unwrap();
        assert_eq!(
            t.release_and_wake_next(id, owner),
            LwMutexRelease::Transferred { new_owner: w1 },
        );
        // Transfer does not signal; the wake hands off directly.
        assert!(!t.lookup(id).unwrap().signaled());
        let remaining: Vec<_> = t.lookup(id).unwrap().waiters().iter().collect();
        assert_eq!(remaining, vec![w2]);
    }

    #[test]
    fn release_unknown_id_is_unknown() {
        let mut t = LwMutexTable::new();
        assert_eq!(
            t.release_and_wake_next(99, tid(0x0100_0001)),
            LwMutexRelease::Unknown,
        );
    }

    #[test]
    fn unlock_then_acquire_via_signal() {
        // Unlock against an empty queue sets the signal; the next
        // acquire consumes it without parking.
        let mut t = LwMutexTable::new();
        let id = t.create().unwrap();
        let a = tid(0x0100_0001);
        assert_eq!(t.release_and_wake_next(id, a), LwMutexRelease::Signaled);
        let b = tid(0x0100_0002);
        assert_eq!(
            t.acquire_or_enqueue(id, b),
            LwMutexAcquireOrEnqueue::Acquired,
        );
        assert!(!t.lookup(id).unwrap().signaled());
    }

    #[test]
    fn state_hash_empty_is_stable() {
        let a = LwMutexTable::new();
        let b = LwMutexTable::new();
        assert_eq!(a.state_hash(), b.state_hash());
    }

    #[test]
    fn state_hash_distinguishes_signaled_state() {
        let mut a = LwMutexTable::new();
        let mut b = LwMutexTable::new();
        let id_a = a.create().unwrap();
        b.create().unwrap();
        assert_eq!(a.state_hash(), b.state_hash());
        // Unlock-with-no-waiters sets the signal on `a` only.
        a.release_and_wake_next(id_a, tid(0x0100_0001));
        assert_ne!(a.state_hash(), b.state_hash());
    }

    #[test]
    fn state_hash_distinguishes_waiter_order() {
        let mut a = LwMutexTable::new();
        let mut b = LwMutexTable::new();
        let id_a = a.create().unwrap();
        let id_b = b.create().unwrap();
        a.try_acquire(id_a, tid(0x0100_0001));
        b.try_acquire(id_b, tid(0x0100_0001));
        a.enqueue_waiter(id_a, tid(0x0100_0002)).unwrap();
        a.enqueue_waiter(id_a, tid(0x0100_0003)).unwrap();
        b.enqueue_waiter(id_b, tid(0x0100_0003)).unwrap();
        b.enqueue_waiter(id_b, tid(0x0100_0002)).unwrap();
        assert_ne!(a.state_hash(), b.state_hash());
    }

    #[test]
    fn state_hash_distinguishes_allocator_cursor() {
        let mut a = LwMutexTable::new();
        let mut b = LwMutexTable::new();
        a.create().unwrap();
        let a_temp = a.create().unwrap();
        a.destroy(a_temp);
        b.create().unwrap();
        assert_eq!(a.len(), b.len());
        assert_ne!(a.state_hash(), b.state_hash());
    }

    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "duplicate enqueue")]
    fn duplicate_enqueue_panics_in_debug() {
        let mut t = LwMutexTable::new();
        let id = t.create().unwrap();
        let waiter = tid(0x0100_0002);
        t.try_acquire(id, tid(0x0100_0001));
        t.enqueue_waiter(id, waiter).unwrap();
        let _ = t.enqueue_waiter(id, waiter);
    }

    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "destroyed with")]
    fn destroy_with_parked_waiters_panics_in_debug() {
        let mut t = LwMutexTable::new();
        let id = t.create().unwrap();
        t.try_acquire(id, tid(0x0100_0001));
        t.enqueue_waiter(id, tid(0x0100_0002)).unwrap();
        let _ = t.destroy(id);
    }

    #[test]
    #[cfg(not(debug_assertions))]
    fn duplicate_enqueue_returns_err_in_release() {
        let mut t = LwMutexTable::new();
        let id = t.create().unwrap();
        let waiter = tid(0x0100_0002);
        t.try_acquire(id, tid(0x0100_0001));
        t.enqueue_waiter(id, waiter).unwrap();
        assert_eq!(
            t.enqueue_waiter(id, waiter),
            Err(LwMutexEnqueueError::DuplicateWaiter),
        );
        assert_eq!(t.lookup(id).unwrap().waiters().len(), 1);
    }

    #[test]
    #[cfg(not(debug_assertions))]
    fn destroy_with_parked_waiters_returns_entry_unchanged_in_release() {
        let mut t = LwMutexTable::new();
        let id = t.create().unwrap();
        let waiter1 = tid(0x0100_0002);
        let waiter2 = tid(0x0100_0003);
        t.try_acquire(id, tid(0x0100_0001));
        t.enqueue_waiter(id, waiter1).unwrap();
        t.enqueue_waiter(id, waiter2).unwrap();
        let removed = t.destroy(id).unwrap();
        let parked: Vec<_> = removed.waiters().iter().collect();
        assert_eq!(parked, vec![waiter1, waiter2]);
        assert!(t.lookup(id).is_none());
    }
}
