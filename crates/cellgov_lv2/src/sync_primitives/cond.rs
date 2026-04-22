//! Condition variable table. Owns state for `sys_cond_create` /
//! `_destroy` / `_wait` / `_signal` / `_signal_all` /
//! `_signal_to`.
//!
//! Non-sticky: a `_signal` call with no parked waiter is
//! observably lost. No pending-signal counter is maintained.
//!
//! The two-hop wake protocol (release the associated mutex on
//! `_wait` entry, re-acquire it on wake) lives in the host
//! dispatch layer, where both this table and the mutex tables
//! are in scope. This table only parks and dequeues cond
//! waiters.

use crate::dispatch::CondMutexKind;
use crate::ppu_thread::PpuThreadId;
use crate::sync_primitives::WaiterList;
use std::collections::BTreeMap;

/// A single condition variable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CondEntry {
    mutex_id: u32,
    mutex_kind: CondMutexKind,
    waiters: WaiterList,
}

impl CondEntry {
    fn new(mutex_id: u32, mutex_kind: CondMutexKind) -> Self {
        Self {
            mutex_id,
            mutex_kind,
            waiters: WaiterList::new(),
        }
    }

    /// Guest id of the associated mutex.
    pub fn mutex_id(&self) -> u32 {
        self.mutex_id
    }

    /// Which mutex table the associated mutex lives in. The
    /// heavy and lightweight mutex tables have distinct id
    /// spaces, so the wake-side re-acquire needs the kind to
    /// route correctly.
    pub fn mutex_kind(&self) -> CondMutexKind {
        self.mutex_kind
    }

    /// Read-only view of the waiter list.
    pub fn waiters(&self) -> &WaiterList {
        &self.waiters
    }
}

/// Failure modes of [`CondTable::create_with_id`]. Both variants
/// indicate a dispatch-layer bug; `debug_assert!` fires before
/// the error is returned.
///
/// The variants split by severity in release: re-registering an
/// identical binding leaves the table unchanged (harmless);
/// re-registering a different binding keeps the existing entry
/// and discards the caller's new binding, which can silently
/// route future operations to the wrong cond.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CondCreateError {
    /// Same id, same binding. Release behavior: no-op.
    RedundantRegistration,
    /// Same id, different binding. Release behavior: existing
    /// entry wins, caller's binding dropped.
    IdCollision {
        /// `mutex_id` of the pre-existing entry.
        existing_mutex_id: u32,
        /// `mutex_kind` of the pre-existing entry.
        existing_mutex_kind: CondMutexKind,
    },
}

/// Failure modes of [`CondTable::enqueue_waiter`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CondEnqueueError {
    /// No cond with this id.
    UnknownId,
    /// Thread is already parked on this cond. A single PPU
    /// thread cannot be in two `sys_cond_wait` syscalls at once;
    /// dispatch-layer bug. `debug_assert!` fires.
    DuplicateWaiter,
}

/// Failure modes of [`CondTable::signal_to`]. The PS3
/// `sys_cond_signal_to` ABI distinguishes these two cases.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CondSignalToError {
    /// No cond with this id.
    UnknownId,
    /// Cond exists but `target` is not on the waiter list.
    TargetNotWaiting,
}

/// Table of condition variables.
#[derive(Debug, Clone, Default)]
pub struct CondTable {
    entries: BTreeMap<u32, CondEntry>,
}

impl CondTable {
    /// Construct an empty table.
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert a fresh entry bound to `mutex_id` / `mutex_kind`.
    /// See [`CondCreateError`].
    pub fn create_with_id(
        &mut self,
        id: u32,
        mutex_id: u32,
        mutex_kind: CondMutexKind,
    ) -> Result<(), CondCreateError> {
        if let Some(existing) = self.entries.get(&id) {
            if existing.mutex_id == mutex_id && existing.mutex_kind == mutex_kind {
                debug_assert!(
                    false,
                    "cond {:#x} redundantly registered (mutex_id={} kind={:?})",
                    id, mutex_id, mutex_kind,
                );
                return Err(CondCreateError::RedundantRegistration);
            }
            debug_assert!(
                false,
                "cond {:#x} collides: new (mutex_id={} kind={:?}) vs existing (mutex_id={} kind={:?})",
                id, mutex_id, mutex_kind, existing.mutex_id, existing.mutex_kind,
            );
            return Err(CondCreateError::IdCollision {
                existing_mutex_id: existing.mutex_id,
                existing_mutex_kind: existing.mutex_kind,
            });
        }
        self.entries
            .insert(id, CondEntry::new(mutex_id, mutex_kind));
        Ok(())
    }

    /// Destroy a cond and return the removed entry, or `None`
    /// if the id was unknown.
    ///
    /// Caller contract: reject non-empty-waiters before calling.
    /// `debug_assert!` fires on violation. If bypassed in
    /// release, the returned entry carries the waiter list and
    /// callers **must** drain and wake each parked thread;
    /// skipping this strands them forever.
    pub fn destroy(&mut self, id: u32) -> Option<CondEntry> {
        let entry = self.entries.remove(&id)?;
        debug_assert!(
            entry.waiters.is_empty(),
            "cond {:#x} destroyed with {} parked waiter(s)",
            id,
            entry.waiters.len(),
        );
        Some(entry)
    }

    /// Read-only lookup.
    pub fn lookup(&self, id: u32) -> Option<&CondEntry> {
        self.entries.get(&id)
    }

    /// Number of tracked conds.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the table has no entries.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Enqueue a waiter. See [`CondEnqueueError`].
    pub fn enqueue_waiter(&mut self, id: u32, waiter: PpuThreadId) -> Result<(), CondEnqueueError> {
        let entry = self
            .entries
            .get_mut(&id)
            .ok_or(CondEnqueueError::UnknownId)?;
        if entry.waiters.enqueue(waiter).is_err() {
            debug_assert!(false, "duplicate enqueue of {:?} on cond {:#x}", waiter, id,);
            return Err(CondEnqueueError::DuplicateWaiter);
        }
        Ok(())
    }

    /// Pop the head of the waiter list. `None` if the cond is
    /// unknown or empty (non-sticky: no pending-signal state).
    /// Used by `sys_cond_signal`.
    pub fn signal_one(&mut self, id: u32) -> Option<PpuThreadId> {
        let entry = self.entries.get_mut(&id)?;
        entry.waiters.dequeue_one()
    }

    /// Drain all waiters in FIFO order. Used by
    /// `sys_cond_signal_all`.
    ///
    /// `None` means the cond is unknown; `Some(empty)` means
    /// the cond exists but has no waiters. The distinction
    /// matters at the ABI level.
    pub fn signal_all(&mut self, id: u32) -> Option<Vec<PpuThreadId>> {
        let entry = self.entries.get_mut(&id)?;
        Some(entry.waiters.drain_all().collect())
    }

    /// Remove `target` from the waiter list. See
    /// [`CondSignalToError`]. Used by `sys_cond_signal_to`.
    pub fn signal_to(&mut self, id: u32, target: PpuThreadId) -> Result<(), CondSignalToError> {
        let entry = self
            .entries
            .get_mut(&id)
            .ok_or(CondSignalToError::UnknownId)?;
        if entry.waiters.remove(target) {
            Ok(())
        } else {
            Err(CondSignalToError::TargetNotWaiting)
        }
    }

    /// FNV-1a digest for state-hash folding. Walks entries in
    /// ascending-id order; folds mutex id, mutex-kind tag, and
    /// the waiter list.
    pub fn state_hash(&self) -> u64 {
        let mut hasher = cellgov_mem::Fnv1aHasher::new();
        hasher.write(&(self.entries.len() as u64).to_le_bytes());
        for (id, entry) in &self.entries {
            hasher.write(&id.to_le_bytes());
            hasher.write(&entry.mutex_id.to_le_bytes());
            let kind_tag: u8 = match entry.mutex_kind {
                CondMutexKind::LwMutex => 0,
                CondMutexKind::Mutex => 1,
            };
            hasher.write(&[kind_tag]);
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
        let t = CondTable::new();
        assert!(t.is_empty());
        assert_eq!(t.len(), 0);
        assert!(t.lookup(1).is_none());
    }

    #[test]
    fn create_with_id_stores_binding() {
        let mut t = CondTable::new();
        assert_eq!(
            t.create_with_id(0x4000_0010, 7, CondMutexKind::LwMutex),
            Ok(())
        );
        let e = t.lookup(0x4000_0010).unwrap();
        assert_eq!(e.mutex_id(), 7);
        assert_eq!(e.mutex_kind(), CondMutexKind::LwMutex);
        assert!(e.waiters().is_empty());
    }

    #[cfg(debug_assertions)]
    #[test]
    #[should_panic(expected = "redundantly registered")]
    fn create_with_id_redundant_registration_fires_debug_assert() {
        let mut t = CondTable::new();
        t.create_with_id(5, 1, CondMutexKind::LwMutex).unwrap();
        let _ = t.create_with_id(5, 1, CondMutexKind::LwMutex);
    }

    #[cfg(debug_assertions)]
    #[test]
    #[should_panic(expected = "collides")]
    fn create_with_id_id_collision_fires_debug_assert() {
        let mut t = CondTable::new();
        t.create_with_id(5, 1, CondMutexKind::LwMutex).unwrap();
        let _ = t.create_with_id(5, 2, CondMutexKind::Mutex);
    }

    #[cfg(not(debug_assertions))]
    #[test]
    fn create_with_id_redundant_registration_returns_err_in_release() {
        let mut t = CondTable::new();
        t.create_with_id(5, 1, CondMutexKind::LwMutex).unwrap();
        assert_eq!(
            t.create_with_id(5, 1, CondMutexKind::LwMutex),
            Err(CondCreateError::RedundantRegistration)
        );
        let e = t.lookup(5).unwrap();
        assert_eq!(e.mutex_id(), 1);
        assert_eq!(e.mutex_kind(), CondMutexKind::LwMutex);
    }

    #[cfg(not(debug_assertions))]
    #[test]
    fn create_with_id_id_collision_returns_err_in_release() {
        let mut t = CondTable::new();
        t.create_with_id(5, 1, CondMutexKind::LwMutex).unwrap();
        assert_eq!(
            t.create_with_id(5, 2, CondMutexKind::Mutex),
            Err(CondCreateError::IdCollision {
                existing_mutex_id: 1,
                existing_mutex_kind: CondMutexKind::LwMutex,
            })
        );
        // Existing binding preserved; caller's new binding dropped.
        let e = t.lookup(5).unwrap();
        assert_eq!(e.mutex_id(), 1);
        assert_eq!(e.mutex_kind(), CondMutexKind::LwMutex);
    }

    #[cfg(not(debug_assertions))]
    #[test]
    fn enqueue_waiter_duplicate_returns_err_in_release() {
        let mut t = CondTable::new();
        t.create_with_id(5, 1, CondMutexKind::LwMutex).unwrap();
        t.enqueue_waiter(5, tid(0x0100_0001)).unwrap();
        assert_eq!(
            t.enqueue_waiter(5, tid(0x0100_0001)),
            Err(CondEnqueueError::DuplicateWaiter)
        );
        assert_eq!(t.lookup(5).unwrap().waiters().len(), 1);
    }

    #[test]
    fn destroy_removes_entry() {
        let mut t = CondTable::new();
        t.create_with_id(5, 1, CondMutexKind::LwMutex).unwrap();
        assert!(t.destroy(5).is_some());
        assert!(t.lookup(5).is_none());
        assert!(t.destroy(5).is_none());
    }

    #[cfg(debug_assertions)]
    #[test]
    #[should_panic(expected = "destroyed with")]
    fn destroy_with_parked_waiters_fires_debug_assert() {
        let mut t = CondTable::new();
        t.create_with_id(5, 1, CondMutexKind::LwMutex).unwrap();
        t.enqueue_waiter(5, tid(0x0100_0001)).unwrap();
        let _ = t.destroy(5);
    }

    #[test]
    fn enqueue_waiter_fifo_order() {
        let mut t = CondTable::new();
        t.create_with_id(5, 1, CondMutexKind::LwMutex).unwrap();
        assert_eq!(t.enqueue_waiter(5, tid(0x0100_0001)), Ok(()));
        assert_eq!(t.enqueue_waiter(5, tid(0x0100_0002)), Ok(()));
        assert_eq!(t.enqueue_waiter(5, tid(0x0100_0003)), Ok(()));
        let seen: Vec<_> = t.lookup(5).unwrap().waiters().iter().collect();
        assert_eq!(
            seen,
            vec![tid(0x0100_0001), tid(0x0100_0002), tid(0x0100_0003)],
        );
    }

    #[test]
    fn enqueue_waiter_unknown_id_returns_unknown_error() {
        let mut t = CondTable::new();
        assert_eq!(
            t.enqueue_waiter(99, tid(0x0100_0001)),
            Err(CondEnqueueError::UnknownId)
        );
    }

    #[cfg(debug_assertions)]
    #[test]
    #[should_panic(expected = "duplicate enqueue")]
    fn enqueue_waiter_duplicate_fires_debug_assert() {
        let mut t = CondTable::new();
        t.create_with_id(5, 1, CondMutexKind::LwMutex).unwrap();
        t.enqueue_waiter(5, tid(0x0100_0001)).unwrap();
        let _ = t.enqueue_waiter(5, tid(0x0100_0001));
    }

    #[test]
    fn signal_one_pops_head() {
        let mut t = CondTable::new();
        t.create_with_id(5, 1, CondMutexKind::LwMutex).unwrap();
        t.enqueue_waiter(5, tid(0x0100_0001)).unwrap();
        t.enqueue_waiter(5, tid(0x0100_0002)).unwrap();
        t.enqueue_waiter(5, tid(0x0100_0003)).unwrap();
        assert_eq!(t.signal_one(5), Some(tid(0x0100_0001)));
        assert_eq!(t.signal_one(5), Some(tid(0x0100_0002)));
        assert_eq!(t.signal_one(5), Some(tid(0x0100_0003)));
        assert_eq!(t.signal_one(5), None);
    }

    #[test]
    fn signal_one_with_no_waiters_is_lost() {
        // Non-sticky: lost signals produce no pending state; a
        // later enqueue still parks the waiter.
        let mut t = CondTable::new();
        t.create_with_id(5, 1, CondMutexKind::LwMutex).unwrap();
        assert_eq!(t.signal_one(5), None);
        assert_eq!(t.signal_one(5), None);
        t.enqueue_waiter(5, tid(0x0100_0001)).unwrap();
        assert_eq!(t.lookup(5).unwrap().waiters().len(), 1);
    }

    #[test]
    fn signal_one_unknown_id_returns_none() {
        let mut t = CondTable::new();
        assert_eq!(t.signal_one(99), None);
    }

    #[test]
    fn signal_all_drains_in_fifo_order() {
        let mut t = CondTable::new();
        t.create_with_id(5, 1, CondMutexKind::LwMutex).unwrap();
        t.enqueue_waiter(5, tid(0x0100_0001)).unwrap();
        t.enqueue_waiter(5, tid(0x0100_0002)).unwrap();
        t.enqueue_waiter(5, tid(0x0100_0003)).unwrap();
        let woken = t.signal_all(5).unwrap();
        assert_eq!(
            woken,
            vec![tid(0x0100_0001), tid(0x0100_0002), tid(0x0100_0003)],
        );
        assert!(t.lookup(5).unwrap().waiters().is_empty());
    }

    #[test]
    fn signal_all_with_no_waiters_returns_some_empty() {
        let mut t = CondTable::new();
        t.create_with_id(5, 1, CondMutexKind::LwMutex).unwrap();
        assert_eq!(t.signal_all(5), Some(Vec::new()));
    }

    #[test]
    fn signal_all_unknown_id_returns_none() {
        let mut t = CondTable::new();
        assert_eq!(t.signal_all(99), None);
    }

    #[test]
    fn signal_to_removes_specific_waiter() {
        let mut t = CondTable::new();
        t.create_with_id(5, 1, CondMutexKind::LwMutex).unwrap();
        t.enqueue_waiter(5, tid(0x0100_0001)).unwrap();
        t.enqueue_waiter(5, tid(0x0100_0002)).unwrap();
        t.enqueue_waiter(5, tid(0x0100_0003)).unwrap();
        assert_eq!(t.signal_to(5, tid(0x0100_0002)), Ok(()));
        let remaining: Vec<_> = t.lookup(5).unwrap().waiters().iter().collect();
        assert_eq!(remaining, vec![tid(0x0100_0001), tid(0x0100_0003)]);
    }

    #[test]
    fn signal_to_missing_target_returns_target_not_waiting() {
        let mut t = CondTable::new();
        t.create_with_id(5, 1, CondMutexKind::LwMutex).unwrap();
        t.enqueue_waiter(5, tid(0x0100_0001)).unwrap();
        assert_eq!(
            t.signal_to(5, tid(0x0100_0099)),
            Err(CondSignalToError::TargetNotWaiting)
        );
    }

    #[test]
    fn signal_to_unknown_id_returns_unknown() {
        let mut t = CondTable::new();
        assert_eq!(
            t.signal_to(99, tid(0x0100_0001)),
            Err(CondSignalToError::UnknownId)
        );
    }

    #[test]
    fn heavy_mutex_binding_is_preserved() {
        let mut t = CondTable::new();
        t.create_with_id(0x4000_0020, 0x4000_0005, CondMutexKind::Mutex)
            .unwrap();
        let e = t.lookup(0x4000_0020).unwrap();
        assert_eq!(e.mutex_kind(), CondMutexKind::Mutex);
        assert_eq!(e.mutex_id(), 0x4000_0005);
    }

    #[test]
    fn state_hash_empty_is_stable() {
        let a = CondTable::new();
        let b = CondTable::new();
        assert_eq!(a.state_hash(), b.state_hash());
    }

    #[test]
    fn state_hash_distinguishes_mutex_kind() {
        let mut a = CondTable::new();
        let mut b = CondTable::new();
        a.create_with_id(5, 1, CondMutexKind::LwMutex).unwrap();
        b.create_with_id(5, 1, CondMutexKind::Mutex).unwrap();
        assert_ne!(a.state_hash(), b.state_hash());
    }

    #[test]
    fn state_hash_distinguishes_mutex_id() {
        let mut a = CondTable::new();
        let mut b = CondTable::new();
        a.create_with_id(5, 1, CondMutexKind::LwMutex).unwrap();
        b.create_with_id(5, 2, CondMutexKind::LwMutex).unwrap();
        assert_ne!(a.state_hash(), b.state_hash());
    }

    #[test]
    fn state_hash_distinguishes_waiter_order() {
        let mut a = CondTable::new();
        let mut b = CondTable::new();
        a.create_with_id(5, 1, CondMutexKind::LwMutex).unwrap();
        b.create_with_id(5, 1, CondMutexKind::LwMutex).unwrap();
        a.enqueue_waiter(5, tid(0x0100_0001)).unwrap();
        a.enqueue_waiter(5, tid(0x0100_0002)).unwrap();
        b.enqueue_waiter(5, tid(0x0100_0002)).unwrap();
        b.enqueue_waiter(5, tid(0x0100_0001)).unwrap();
        assert_ne!(a.state_hash(), b.state_hash());
    }

    #[test]
    fn state_hash_ignores_ephemeral_signal_attempts() {
        // Pins the non-sticky contract at the hash level: lost
        // signals leave no trace.
        let mut a = CondTable::new();
        let mut b = CondTable::new();
        a.create_with_id(5, 1, CondMutexKind::LwMutex).unwrap();
        b.create_with_id(5, 1, CondMutexKind::LwMutex).unwrap();
        let _ = a.signal_one(5);
        let _ = a.signal_all(5);
        let _ = a.signal_to(5, tid(0x0100_0099));
        assert_eq!(a.state_hash(), b.state_hash());
    }
}
