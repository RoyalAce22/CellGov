//! Heavy mutex table.
//!
//! Ids come from the shared kernel-object allocator, distinct
//! from the lwmutex id space. Attributes captured from
//! `sys_mutex_create` are stored and hashed but never honored;
//! see [`MutexAttrs`].

use crate::ppu_thread::PpuThreadId;
use crate::sync_primitives::WaiterList;
use std::collections::BTreeMap;

/// Outcome of a `try_acquire` call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MutexAcquire {
    /// Caller is now the owner.
    Acquired,
    /// Mutex is owned.
    Contended,
}

/// Outcome of an `acquire_or_enqueue` call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MutexAcquireOrEnqueue {
    /// Caller is now the owner.
    Acquired,
    /// Caller was appended to the waiter list.
    Enqueued,
    /// Caller already holds the mutex or is already parked.
    /// Non-recursive regardless of [`MutexAttrs::recursive`].
    WouldDeadlock,
    /// Unknown id.
    Unknown,
}

/// Outcome of a `release_and_wake_next` call.
#[must_use = "ignoring a MutexRelease drops the wake-up for any transferred owner"]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MutexRelease {
    /// Mutex is now unowned; no waiter was woken.
    Freed,
    /// Ownership transferred to `new_owner`; caller must wake it.
    Transferred {
        /// Thread that just became the owner.
        new_owner: PpuThreadId,
    },
    /// Caller did not own the mutex.
    NotOwner,
    /// Unknown id.
    Unknown,
}

/// Failure modes of [`MutexTable::create_with_id`].
///
/// `IdCollision` indicates an allocator bug; `debug_assert!`
/// fires. Release keeps the existing entry and returns `Err`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MutexCreateError {
    /// An entry with this id was already present.
    IdCollision,
}

/// Failure modes of [`MutexTable::enqueue_waiter`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MutexEnqueueError {
    /// No mutex with this id.
    UnknownId,
    /// Thread is already on the waiter list. Always state
    /// corruption; callers route to `record_invariant_break`.
    DuplicateWaiter,
    /// Thread is the current owner. Reachable from guest
    /// recursive-lock attempts.
    WaiterIsOwner,
}

/// Attribute bag captured from `sys_mutex_create`. No field
/// affects blocking or waking; recursive locks surface as
/// `WouldDeadlock` / `WaiterIsOwner` regardless of `recursive`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MutexAttrs {
    /// Priority-ordering policy; diagnostic only.
    pub priority_policy: u32,
    /// Recursive flag; not honored.
    pub recursive: bool,
    /// Raw protocol bits; diagnostic only.
    pub protocol: u32,
}

/// A single heavy mutex.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MutexEntry {
    owner: Option<PpuThreadId>,
    waiters: WaiterList,
    attrs: MutexAttrs,
}

impl MutexEntry {
    fn new(attrs: MutexAttrs) -> Self {
        Self {
            owner: None,
            waiters: WaiterList::new(),
            attrs,
        }
    }

    /// Current owner, or `None` if free.
    pub fn owner(&self) -> Option<PpuThreadId> {
        self.owner
    }

    /// Read-only view of the waiter list.
    pub fn waiters(&self) -> &WaiterList {
        &self.waiters
    }

    /// Attributes captured at create time.
    pub fn attrs(&self) -> MutexAttrs {
        self.attrs
    }
}

/// Table of heavy mutexes.
#[derive(Debug, Clone, Default)]
pub struct MutexTable {
    entries: BTreeMap<u32, MutexEntry>,
}

impl MutexTable {
    /// Construct an empty table.
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert a fresh entry. See [`MutexCreateError`].
    pub fn create_with_id(&mut self, id: u32, attrs: MutexAttrs) -> Result<(), MutexCreateError> {
        if let Some(existing) = self.entries.get(&id) {
            debug_assert!(
                false,
                "mutex id {:#x} already present (existing {:?} owner={:?}, new {:?})",
                id, existing.attrs, existing.owner, attrs,
            );
            return Err(MutexCreateError::IdCollision);
        }
        self.entries.insert(id, MutexEntry::new(attrs));
        Ok(())
    }

    /// Remove the entry; `None` if the id was unknown.
    ///
    /// Caller contract: reject held or non-empty-waiters before
    /// calling (`debug_assert!`s fire on violation). If bypassed
    /// in release, callers **must** drain `entry.waiters()` and
    /// wake each parked thread; skipping this strands them
    /// forever.
    pub fn destroy(&mut self, id: u32) -> Option<MutexEntry> {
        let entry = self.entries.remove(&id)?;
        debug_assert!(
            entry.owner.is_none(),
            "mutex {:#x} destroyed while held by {:?}",
            id,
            entry.owner,
        );
        debug_assert!(
            entry.waiters.is_empty(),
            "mutex {:#x} destroyed with {} parked waiter(s)",
            id,
            entry.waiters.len(),
        );
        Some(entry)
    }

    /// Read-only lookup.
    pub fn lookup(&self, id: u32) -> Option<&MutexEntry> {
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

    /// Check-and-set without enqueueing. Non-recursive: the
    /// owner re-acquiring sees `Contended`, not `WouldDeadlock`.
    pub fn try_acquire(&mut self, id: u32, caller: PpuThreadId) -> Option<MutexAcquire> {
        let entry = self.entries.get_mut(&id)?;
        if entry.owner.is_none() {
            entry.owner = Some(caller);
            Some(MutexAcquire::Acquired)
        } else {
            Some(MutexAcquire::Contended)
        }
    }

    /// Atomic acquire-or-park.
    ///
    /// O(n) scan over the waiter list on the already-parked
    /// check; defensive (normal dispatch cannot reach it).
    pub fn acquire_or_enqueue(&mut self, id: u32, caller: PpuThreadId) -> MutexAcquireOrEnqueue {
        let Some(entry) = self.entries.get_mut(&id) else {
            return MutexAcquireOrEnqueue::Unknown;
        };
        match entry.owner {
            None => {
                entry.owner = Some(caller);
                MutexAcquireOrEnqueue::Acquired
            }
            Some(owner) if owner == caller => MutexAcquireOrEnqueue::WouldDeadlock,
            Some(_) => {
                if entry.waiters.contains(caller) {
                    return MutexAcquireOrEnqueue::WouldDeadlock;
                }
                // Contains check above rules out duplicate.
                if entry.waiters.enqueue(caller).is_err() {
                    debug_assert!(
                        false,
                        "contains guard broken for mutex {id:#x} caller {caller:?}"
                    );
                }
                MutexAcquireOrEnqueue::Enqueued
            }
        }
    }

    /// Enqueue `waiter` on the mutex's waiter list.
    ///
    /// # Errors
    /// - [`MutexEnqueueError::UnknownId`] if `id` is absent.
    /// - [`MutexEnqueueError::WaiterIsOwner`] if `waiter` holds
    ///   the mutex.
    /// - [`MutexEnqueueError::DuplicateWaiter`] if `waiter` is
    ///   already parked; callers must route to
    ///   `record_invariant_break`.
    pub fn enqueue_waiter(
        &mut self,
        id: u32,
        waiter: PpuThreadId,
    ) -> Result<(), MutexEnqueueError> {
        let entry = self
            .entries
            .get_mut(&id)
            .ok_or(MutexEnqueueError::UnknownId)?;
        if entry.owner == Some(waiter) {
            return Err(MutexEnqueueError::WaiterIsOwner);
        }
        if entry.waiters.enqueue(waiter).is_err() {
            return Err(MutexEnqueueError::DuplicateWaiter);
        }
        Ok(())
    }

    /// Release on behalf of `caller`.
    pub fn release_and_wake_next(&mut self, id: u32, caller: PpuThreadId) -> MutexRelease {
        let Some(entry) = self.entries.get_mut(&id) else {
            return MutexRelease::Unknown;
        };
        if entry.owner != Some(caller) {
            return MutexRelease::NotOwner;
        }
        match entry.waiters.dequeue_one() {
            Some(new_owner) => {
                entry.owner = Some(new_owner);
                MutexRelease::Transferred { new_owner }
            }
            None => {
                entry.owner = None;
                MutexRelease::Freed
            }
        }
    }

    /// FNV-1a digest of the table's state, including attrs.
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
            hasher.write(&entry.attrs.priority_policy.to_le_bytes());
            hasher.write(&[entry.attrs.recursive as u8]);
            hasher.write(&entry.attrs.protocol.to_le_bytes());
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

    fn default_attrs() -> MutexAttrs {
        MutexAttrs::default()
    }

    #[test]
    fn fresh_table_is_empty() {
        let t = MutexTable::new();
        assert!(t.is_empty());
        assert_eq!(t.len(), 0);
        assert!(t.lookup(1).is_none());
    }

    #[test]
    fn try_acquire_unowned_sets_owner() {
        let mut t = MutexTable::new();
        t.create_with_id(1, default_attrs()).unwrap();
        assert_eq!(
            t.try_acquire(1, tid(0x0100_0001)),
            Some(MutexAcquire::Acquired),
        );
        assert_eq!(t.lookup(1).unwrap().owner(), Some(tid(0x0100_0001)));
    }

    #[test]
    fn try_acquire_contended_does_not_change_owner() {
        let mut t = MutexTable::new();
        t.create_with_id(1, default_attrs()).unwrap();
        let a = tid(0x0100_0001);
        let b = tid(0x0100_0002);
        t.try_acquire(1, a);
        assert_eq!(t.try_acquire(1, b), Some(MutexAcquire::Contended));
        assert_eq!(t.lookup(1).unwrap().owner(), Some(a));
    }

    #[test]
    fn try_acquire_same_thread_twice_is_contended() {
        let mut t = MutexTable::new();
        t.create_with_id(1, default_attrs()).unwrap();
        let a = tid(0x0100_0001);
        t.try_acquire(1, a);
        assert_eq!(t.try_acquire(1, a), Some(MutexAcquire::Contended));
    }

    #[test]
    fn acquire_or_enqueue_unowned_sets_owner() {
        let mut t = MutexTable::new();
        t.create_with_id(1, default_attrs()).unwrap();
        let a = tid(0x0100_0001);
        assert_eq!(t.acquire_or_enqueue(1, a), MutexAcquireOrEnqueue::Acquired,);
        assert_eq!(t.lookup(1).unwrap().owner(), Some(a));
    }

    #[test]
    fn acquire_or_enqueue_enqueues_contender() {
        let mut t = MutexTable::new();
        t.create_with_id(1, default_attrs()).unwrap();
        let owner = tid(0x0100_0001);
        let contender = tid(0x0100_0002);
        t.acquire_or_enqueue(1, owner);
        assert_eq!(
            t.acquire_or_enqueue(1, contender),
            MutexAcquireOrEnqueue::Enqueued,
        );
        let parked: Vec<_> = t.lookup(1).unwrap().waiters().iter().collect();
        assert_eq!(parked, vec![contender]);
    }

    #[test]
    fn acquire_or_enqueue_owner_retrying_is_would_deadlock() {
        let mut t = MutexTable::new();
        t.create_with_id(1, default_attrs()).unwrap();
        let a = tid(0x0100_0001);
        t.acquire_or_enqueue(1, a);
        assert_eq!(
            t.acquire_or_enqueue(1, a),
            MutexAcquireOrEnqueue::WouldDeadlock,
        );
        assert_eq!(t.lookup(1).unwrap().owner(), Some(a));
        assert!(t.lookup(1).unwrap().waiters().is_empty());
    }

    #[test]
    fn acquire_or_enqueue_already_parked_is_would_deadlock() {
        let mut t = MutexTable::new();
        t.create_with_id(1, default_attrs()).unwrap();
        let owner = tid(0x0100_0001);
        let waiter = tid(0x0100_0002);
        t.acquire_or_enqueue(1, owner);
        assert_eq!(
            t.acquire_or_enqueue(1, waiter),
            MutexAcquireOrEnqueue::Enqueued,
        );
        assert_eq!(
            t.acquire_or_enqueue(1, waiter),
            MutexAcquireOrEnqueue::WouldDeadlock,
        );
        assert_eq!(t.lookup(1).unwrap().waiters().len(), 1);
    }

    #[test]
    fn acquire_or_enqueue_ignores_recursive_attr() {
        let attrs = MutexAttrs {
            recursive: true,
            ..Default::default()
        };
        let mut t = MutexTable::new();
        t.create_with_id(1, attrs).unwrap();
        let a = tid(0x0100_0001);
        t.acquire_or_enqueue(1, a);
        assert_eq!(
            t.acquire_or_enqueue(1, a),
            MutexAcquireOrEnqueue::WouldDeadlock,
        );
    }

    #[test]
    fn acquire_or_enqueue_unknown_id_is_unknown() {
        let mut t = MutexTable::new();
        assert_eq!(
            t.acquire_or_enqueue(99, tid(0x0100_0001)),
            MutexAcquireOrEnqueue::Unknown,
        );
    }

    #[test]
    fn enqueue_waiter_preserves_fifo_order() {
        let mut t = MutexTable::new();
        t.create_with_id(1, default_attrs()).unwrap();
        t.try_acquire(1, tid(0x0100_0001));
        t.enqueue_waiter(1, tid(0x0100_0002)).unwrap();
        t.enqueue_waiter(1, tid(0x0100_0003)).unwrap();
        let seen: Vec<_> = t.lookup(1).unwrap().waiters().iter().collect();
        assert_eq!(seen, vec![tid(0x0100_0002), tid(0x0100_0003)]);
    }

    #[test]
    fn enqueue_waiter_unknown_id_returns_err() {
        let mut t = MutexTable::new();
        assert_eq!(
            t.enqueue_waiter(99, tid(0x0100_0001)),
            Err(MutexEnqueueError::UnknownId),
        );
    }

    #[test]
    fn enqueue_waiter_duplicate_returns_err() {
        let mut t = MutexTable::new();
        t.create_with_id(1, default_attrs()).unwrap();
        let owner = tid(0x0100_0001);
        let waker = tid(0x0100_0002);
        t.try_acquire(1, owner);
        t.enqueue_waiter(1, waker).unwrap();
        assert_eq!(
            t.enqueue_waiter(1, waker),
            Err(MutexEnqueueError::DuplicateWaiter),
        );
        assert_eq!(t.lookup(1).unwrap().waiters().len(), 1);
    }

    #[test]
    fn enqueue_waiter_on_owner_returns_err() {
        let mut t = MutexTable::new();
        t.create_with_id(1, default_attrs()).unwrap();
        let owner = tid(0x0100_0001);
        t.try_acquire(1, owner);
        assert_eq!(
            t.enqueue_waiter(1, owner),
            Err(MutexEnqueueError::WaiterIsOwner),
        );
        assert_eq!(t.lookup(1).unwrap().owner(), Some(owner));
        assert!(t.lookup(1).unwrap().waiters().is_empty());
    }

    #[test]
    fn release_without_waiters_frees() {
        let mut t = MutexTable::new();
        t.create_with_id(1, default_attrs()).unwrap();
        let a = tid(0x0100_0001);
        t.try_acquire(1, a);
        assert_eq!(t.release_and_wake_next(1, a), MutexRelease::Freed);
        assert_eq!(t.lookup(1).unwrap().owner(), None);
    }

    #[test]
    fn release_with_waiters_transfers_to_head() {
        let mut t = MutexTable::new();
        t.create_with_id(1, default_attrs()).unwrap();
        let owner = tid(0x0100_0001);
        let w1 = tid(0x0100_0002);
        let w2 = tid(0x0100_0003);
        t.try_acquire(1, owner);
        t.enqueue_waiter(1, w1).unwrap();
        t.enqueue_waiter(1, w2).unwrap();
        assert_eq!(
            t.release_and_wake_next(1, owner),
            MutexRelease::Transferred { new_owner: w1 },
        );
        assert_eq!(t.lookup(1).unwrap().owner(), Some(w1));
        let remaining: Vec<_> = t.lookup(1).unwrap().waiters().iter().collect();
        assert_eq!(remaining, vec![w2]);
    }

    #[test]
    fn release_by_non_owner_is_rejected() {
        let mut t = MutexTable::new();
        t.create_with_id(1, default_attrs()).unwrap();
        let a = tid(0x0100_0001);
        let b = tid(0x0100_0002);
        t.try_acquire(1, a);
        assert_eq!(t.release_and_wake_next(1, b), MutexRelease::NotOwner);
        assert_eq!(t.lookup(1).unwrap().owner(), Some(a));
    }

    #[test]
    fn release_unknown_id_is_unknown() {
        let mut t = MutexTable::new();
        assert_eq!(
            t.release_and_wake_next(99, tid(0x0100_0001)),
            MutexRelease::Unknown,
        );
    }

    #[test]
    fn attrs_round_trip() {
        let attrs = MutexAttrs {
            priority_policy: 2,
            recursive: true,
            protocol: 0x20,
        };
        let mut t = MutexTable::new();
        t.create_with_id(1, attrs).unwrap();
        assert_eq!(t.lookup(1).unwrap().attrs(), attrs);
    }

    #[test]
    fn destroy_free_mutex_removes_entry() {
        let mut t = MutexTable::new();
        t.create_with_id(1, default_attrs()).unwrap();
        let removed = t.destroy(1).unwrap();
        assert!(removed.owner().is_none());
        assert!(removed.waiters().is_empty());
        assert!(t.lookup(1).is_none());
    }

    #[test]
    fn destroy_unknown_id_is_none() {
        let mut t = MutexTable::new();
        assert!(t.destroy(99).is_none());
    }

    #[test]
    fn state_hash_distinguishes_attrs() {
        let mut a = MutexTable::new();
        let mut b = MutexTable::new();
        a.create_with_id(
            1,
            MutexAttrs {
                priority_policy: 1,
                ..Default::default()
            },
        )
        .unwrap();
        b.create_with_id(
            1,
            MutexAttrs {
                priority_policy: 2,
                ..Default::default()
            },
        )
        .unwrap();
        assert_ne!(a.state_hash(), b.state_hash());
    }

    #[test]
    fn state_hash_distinguishes_waiter_order() {
        let mut a = MutexTable::new();
        let mut b = MutexTable::new();
        a.create_with_id(1, default_attrs()).unwrap();
        b.create_with_id(1, default_attrs()).unwrap();
        let owner = tid(0x0100_0001);
        a.try_acquire(1, owner);
        b.try_acquire(1, owner);
        a.enqueue_waiter(1, tid(0x0100_0002)).unwrap();
        a.enqueue_waiter(1, tid(0x0100_0003)).unwrap();
        b.enqueue_waiter(1, tid(0x0100_0003)).unwrap();
        b.enqueue_waiter(1, tid(0x0100_0002)).unwrap();
        assert_ne!(a.state_hash(), b.state_hash());
    }

    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "already present")]
    fn create_with_id_collision_panics_in_debug() {
        let mut t = MutexTable::new();
        t.create_with_id(5, default_attrs()).unwrap();
        let _ = t.create_with_id(5, default_attrs());
    }

    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "destroyed while held")]
    fn destroy_held_mutex_panics_in_debug() {
        let mut t = MutexTable::new();
        t.create_with_id(1, default_attrs()).unwrap();
        t.try_acquire(1, tid(0x0100_0001));
        let _ = t.destroy(1);
    }

    #[test]
    #[cfg(not(debug_assertions))]
    fn create_with_id_collision_returns_err_in_release() {
        let mut t = MutexTable::new();
        t.create_with_id(5, default_attrs()).unwrap();
        assert_eq!(
            t.create_with_id(5, default_attrs()),
            Err(MutexCreateError::IdCollision),
        );
        assert_eq!(t.len(), 1);
    }

    #[test]
    #[cfg(not(debug_assertions))]
    fn destroy_held_mutex_returns_entry_unchanged_in_release() {
        let mut t = MutexTable::new();
        t.create_with_id(1, default_attrs()).unwrap();
        let owner = tid(0x0100_0001);
        let waiter = tid(0x0100_0002);
        t.try_acquire(1, owner);
        t.enqueue_waiter(1, waiter).unwrap();
        let removed = t.destroy(1).unwrap();
        assert_eq!(removed.owner(), Some(owner));
        let parked: Vec<_> = removed.waiters().iter().collect();
        assert_eq!(parked, vec![waiter]);
        assert!(t.lookup(1).is_none());
    }
}
