//! Lightweight mutex table.
//!
//! Owns the state for `sys_lwmutex_create` / `_destroy` / `_lock` /
//! `_unlock` / `_trylock`. Each entry is keyed by a guest-visible
//! `u32` id, allocated monotonically by `LwMutexIdAllocator`.
//!
//! Lock semantics are classical mutual exclusion: the table tracks
//! the current owner (if any) and a FIFO waiter list. `try_acquire`
//! succeeds exactly when the mutex is unowned and sets the owner;
//! otherwise the caller is expected to enqueue on the waiter list
//! via `enqueue_waiter` and park. `release_and_wake_next` validates
//! that the releaser is the current owner, pops the head of the
//! waiter list (if any), and transfers ownership in the same call.
//!
//! The table is pure data: no runtime integration, no dispatch
//! wiring. Higher-level handlers in `cellgov_lv2::host` (and the
//! runtime's syscall-response plumbing) compose these primitives
//! into the actual `sys_lwmutex_*` behavior.

use crate::ppu_thread::PpuThreadId;
use crate::sync_primitives::WaiterList;
use std::collections::BTreeMap;

/// Outcome of a `try_acquire` call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LwMutexAcquire {
    /// The caller became the owner. The caller's syscall returns
    /// `CELL_OK` and does not park.
    Acquired,
    /// The mutex is owned by another thread. The caller should
    /// either park (for `_lock`) or return EBUSY (for `_trylock`).
    Contended,
}

/// Outcome of a `release_and_wake_next` call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LwMutexRelease {
    /// The mutex is now unowned. No waiter was woken.
    Freed,
    /// Ownership transferred to `new_owner`. The runtime should wake
    /// that thread and deliver its pending `CELL_OK` response.
    Transferred {
        /// Thread that just became the owner.
        new_owner: PpuThreadId,
    },
    /// The caller did not own the mutex. The release is rejected
    /// and the table is unchanged. The syscall returns `EPERM`.
    NotOwner,
    /// The mutex id is unknown (either never created or already
    /// destroyed). Syscall returns `ESRCH`.
    Unknown,
}

/// A single lightweight mutex.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LwMutexEntry {
    /// Current owner, if any.
    owner: Option<PpuThreadId>,
    /// FIFO list of threads parked on this mutex.
    waiters: WaiterList,
}

impl LwMutexEntry {
    fn new() -> Self {
        Self {
            owner: None,
            waiters: WaiterList::new(),
        }
    }

    /// Current owner (if any). `None` means the mutex is free.
    pub fn owner(&self) -> Option<PpuThreadId> {
        self.owner
    }

    /// Read-only view of the waiter list.
    pub fn waiters(&self) -> &WaiterList {
        &self.waiters
    }
}

/// Monotonic allocator for lwmutex ids. Starts at 1; `u32::MAX` is
/// reserved to signal exhaustion. Two tables constructed identically
/// allocate identical id sequences.
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
    /// Construct a fresh allocator. The first `allocate` returns 1.
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

    /// Allocate a fresh id, create the entry, and return the id.
    /// Returns `None` if the id space is exhausted.
    pub fn create(&mut self) -> Option<u32> {
        let id = self.ids.allocate()?;
        self.entries.insert(id, LwMutexEntry::new());
        Some(id)
    }

    /// Destroy a lwmutex. Returns the removed entry (for diagnostic
    /// inspection in tests) or `None` if the id was unknown. A
    /// destroy while waiters are parked is a guest programming
    /// error; this method does not enforce that -- the handler above
    /// validates and rejects if non-empty.
    pub fn destroy(&mut self, id: u32) -> Option<LwMutexEntry> {
        self.entries.remove(&id)
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

    /// Attempt to acquire `id` on behalf of `caller`.
    ///
    /// Returns `None` if `id` is unknown. Otherwise returns
    /// `Some(Acquired)` when the mutex was unowned (and sets the
    /// owner to `caller`), or `Some(Contended)` when it is owned by
    /// some other thread. Re-acquiring an already-held mutex by the
    /// same caller is rejected with `Contended` -- lwmutex is not
    /// recursive.
    pub fn try_acquire(&mut self, id: u32, caller: PpuThreadId) -> Option<LwMutexAcquire> {
        let entry = self.entries.get_mut(&id)?;
        if entry.owner.is_none() {
            entry.owner = Some(caller);
            Some(LwMutexAcquire::Acquired)
        } else {
            Some(LwMutexAcquire::Contended)
        }
    }

    /// Enqueue `waiter` on the waiter list for `id`. Returns `true`
    /// on success, `false` if `id` is unknown or `waiter` is already
    /// parked on this mutex.
    pub fn enqueue_waiter(&mut self, id: u32, waiter: PpuThreadId) -> bool {
        let Some(entry) = self.entries.get_mut(&id) else {
            return false;
        };
        entry.waiters.enqueue(waiter)
    }

    /// Release `id` on behalf of `caller`. If a waiter is parked,
    /// transfer ownership to the head of the queue and return
    /// `Transferred { new_owner }`. Otherwise clear ownership and
    /// return `Freed`. Returns `NotOwner` if `caller` does not own
    /// the mutex, `Unknown` if `id` is not in the table.
    pub fn release_and_wake_next(&mut self, id: u32, caller: PpuThreadId) -> LwMutexRelease {
        let Some(entry) = self.entries.get_mut(&id) else {
            return LwMutexRelease::Unknown;
        };
        if entry.owner != Some(caller) {
            return LwMutexRelease::NotOwner;
        }
        match entry.waiters.dequeue_one() {
            Some(new_owner) => {
                entry.owner = Some(new_owner);
                LwMutexRelease::Transferred { new_owner }
            }
            None => {
                entry.owner = None;
                LwMutexRelease::Freed
            }
        }
    }

    /// FNV-1a digest of the table for state-hash folding. Walks
    /// entries in BTreeMap (ascending id) order; within each entry
    /// folds owner presence, owner id, and the waiter list in
    /// enqueue order. Tables differing in any of those produce
    /// distinct hashes.
    pub fn state_hash(&self) -> u64 {
        let mut hasher = cellgov_mem::Fnv1aHasher::new();
        hasher.write(&(self.entries.len() as u64).to_le_bytes());
        for (id, entry) in &self.entries {
            hasher.write(&id.to_le_bytes());
            match entry.owner {
                Some(owner) => {
                    hasher.write(&[1u8]);
                    hasher.write(&owner.raw().to_le_bytes());
                }
                None => hasher.write(&[0u8]),
            }
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
        // Stays exhausted.
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
    fn destroy_removes_entry() {
        let mut t = LwMutexTable::new();
        let id = t.create().unwrap();
        assert!(t.destroy(id).is_some());
        assert!(t.lookup(id).is_none());
        assert!(t.destroy(id).is_none());
    }

    #[test]
    fn try_acquire_unowned_sets_owner() {
        let mut t = LwMutexTable::new();
        let id = t.create().unwrap();
        let caller = tid(0x0100_0001);
        assert_eq!(t.try_acquire(id, caller), Some(LwMutexAcquire::Acquired));
        assert_eq!(t.lookup(id).unwrap().owner(), Some(caller));
    }

    #[test]
    fn try_acquire_contended_does_not_change_owner() {
        let mut t = LwMutexTable::new();
        let id = t.create().unwrap();
        let a = tid(0x0100_0001);
        let b = tid(0x0100_0002);
        t.try_acquire(id, a);
        assert_eq!(t.try_acquire(id, b), Some(LwMutexAcquire::Contended));
        assert_eq!(t.lookup(id).unwrap().owner(), Some(a));
    }

    #[test]
    fn try_acquire_same_thread_twice_is_contended() {
        // Lwmutex is not recursive. Re-acquiring by the owner gets
        // Contended, matching the ABI.
        let mut t = LwMutexTable::new();
        let id = t.create().unwrap();
        let a = tid(0x0100_0001);
        t.try_acquire(id, a);
        assert_eq!(t.try_acquire(id, a), Some(LwMutexAcquire::Contended));
    }

    #[test]
    fn try_acquire_unknown_id_is_none() {
        let mut t = LwMutexTable::new();
        assert!(t.try_acquire(99, tid(0x0100_0001)).is_none());
    }

    #[test]
    fn enqueue_waiter_preserves_fifo_order() {
        let mut t = LwMutexTable::new();
        let id = t.create().unwrap();
        let owner = tid(0x0100_0001);
        t.try_acquire(id, owner);
        assert!(t.enqueue_waiter(id, tid(0x0100_0002)));
        assert!(t.enqueue_waiter(id, tid(0x0100_0003)));
        assert!(t.enqueue_waiter(id, tid(0x0100_0004)));
        let seen: Vec<_> = t.lookup(id).unwrap().waiters().iter().collect();
        assert_eq!(
            seen,
            vec![tid(0x0100_0002), tid(0x0100_0003), tid(0x0100_0004)],
        );
    }

    #[test]
    fn enqueue_waiter_unknown_id_returns_false() {
        let mut t = LwMutexTable::new();
        assert!(!t.enqueue_waiter(99, tid(0x0100_0001)));
    }

    #[test]
    fn release_without_waiters_frees() {
        let mut t = LwMutexTable::new();
        let id = t.create().unwrap();
        let a = tid(0x0100_0001);
        t.try_acquire(id, a);
        assert_eq!(t.release_and_wake_next(id, a), LwMutexRelease::Freed);
        assert_eq!(t.lookup(id).unwrap().owner(), None);
    }

    #[test]
    fn release_with_waiters_transfers_to_head() {
        let mut t = LwMutexTable::new();
        let id = t.create().unwrap();
        let owner = tid(0x0100_0001);
        let w1 = tid(0x0100_0002);
        let w2 = tid(0x0100_0003);
        t.try_acquire(id, owner);
        t.enqueue_waiter(id, w1);
        t.enqueue_waiter(id, w2);
        assert_eq!(
            t.release_and_wake_next(id, owner),
            LwMutexRelease::Transferred { new_owner: w1 },
        );
        assert_eq!(t.lookup(id).unwrap().owner(), Some(w1));
        // w2 is still parked.
        let remaining: Vec<_> = t.lookup(id).unwrap().waiters().iter().collect();
        assert_eq!(remaining, vec![w2]);
    }

    #[test]
    fn release_by_non_owner_is_rejected() {
        let mut t = LwMutexTable::new();
        let id = t.create().unwrap();
        let a = tid(0x0100_0001);
        let b = tid(0x0100_0002);
        t.try_acquire(id, a);
        assert_eq!(t.release_and_wake_next(id, b), LwMutexRelease::NotOwner);
        // Owner unchanged.
        assert_eq!(t.lookup(id).unwrap().owner(), Some(a));
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
    fn state_hash_empty_is_stable() {
        let a = LwMutexTable::new();
        let b = LwMutexTable::new();
        assert_eq!(a.state_hash(), b.state_hash());
    }

    #[test]
    fn state_hash_distinguishes_owner_change() {
        let mut a = LwMutexTable::new();
        let mut b = LwMutexTable::new();
        let id_a = a.create().unwrap();
        let id_b = b.create().unwrap();
        assert_eq!(a.state_hash(), b.state_hash());
        a.try_acquire(id_a, tid(0x0100_0001));
        assert_ne!(a.state_hash(), b.state_hash());
        b.try_acquire(id_b, tid(0x0100_0001));
        assert_eq!(a.state_hash(), b.state_hash());
    }

    #[test]
    fn state_hash_distinguishes_waiter_order() {
        let mut a = LwMutexTable::new();
        let mut b = LwMutexTable::new();
        let id_a = a.create().unwrap();
        let id_b = b.create().unwrap();
        let owner = tid(0x0100_0001);
        a.try_acquire(id_a, owner);
        b.try_acquire(id_b, owner);
        a.enqueue_waiter(id_a, tid(0x0100_0002));
        a.enqueue_waiter(id_a, tid(0x0100_0003));
        b.enqueue_waiter(id_b, tid(0x0100_0003));
        b.enqueue_waiter(id_b, tid(0x0100_0002));
        // Same set of waiters, different FIFO order -- must hash
        // distinctly per the determinism contract.
        assert_ne!(a.state_hash(), b.state_hash());
    }

    #[test]
    fn destroy_with_waiters_returns_entry_unchanged() {
        // The handler above validates "cannot destroy with waiters";
        // the table itself is permissive. This test documents that
        // the table does not silently discard or rearrange waiters
        // on destroy.
        let mut t = LwMutexTable::new();
        let id = t.create().unwrap();
        t.try_acquire(id, tid(0x0100_0001));
        t.enqueue_waiter(id, tid(0x0100_0002));
        let removed = t.destroy(id).unwrap();
        assert_eq!(removed.owner(), Some(tid(0x0100_0001)));
        assert_eq!(removed.waiters().len(), 1);
    }
}
