//! Condition variable table.
//!
//! Owns the state for `sys_cond_create` / `_destroy` / `_wait` /
//! `_signal` / `_signal_all` / `_signal_to`. Each entry is keyed by
//! a guest-visible `u32` id minted by the host's shared kernel-object
//! allocator and carries:
//!
//!   * `mutex_id: u32` -- the associated mutex, captured at create
//!     time. `sys_cond_create` binds a cond to exactly one mutex;
//!     `sys_cond_wait` requires the caller to hold that mutex and
//!     releases it on the way in.
//!   * `mutex_kind: CondMutexKind` -- whether the associated mutex
//!     lives in the lwmutex table or the heavy mutex table. They
//!     have distinct id spaces, so the wake-side re-acquire needs
//!     to know which table to consult.
//!   * `waiters: WaiterList` -- FIFO queue of PPU threads parked on
//!     `_wait`.
//!
//! Cond is non-sticky: a `_signal` / `_signal_all` / `_signal_to`
//! call with no matching parked waiter is observably lost. There is
//! no "pending signal count" field, and a later `_wait` will block
//! regardless of how many signals preceded it. This matches the
//! POSIX cond contract.
//!
//! The two-hop wake protocol (drop mutex on wait, re-acquire on
//! wake) is not implemented in this table. The table's job is to
//! park and dequeue cond waiters; the wake-side re-acquire decision
//! (mutex free -> take it, mutex held -> re-park on the mutex's
//! waiter list) lives in the host dispatch layer, where both the
//! cond table and the mutex tables are in scope.

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

    /// Which mutex table the associated mutex lives in.
    pub fn mutex_kind(&self) -> CondMutexKind {
        self.mutex_kind
    }

    /// Read-only view of the waiter list.
    pub fn waiters(&self) -> &WaiterList {
        &self.waiters
    }
}

/// Table of condition variables. Ids come from the host's shared
/// kernel-object allocator; `create_with_id` is the only entry point.
#[derive(Debug, Clone, Default)]
pub struct CondTable {
    entries: BTreeMap<u32, CondEntry>,
}

impl CondTable {
    /// Construct an empty table.
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert a fresh entry with the given id bound to `mutex_id` /
    /// `mutex_kind`. The caller (host) mints the id; this table
    /// stores the entry. Returns `false` if `id` is already present
    /// (allocator collision -- a bug, not a guest-visible error).
    pub fn create_with_id(&mut self, id: u32, mutex_id: u32, mutex_kind: CondMutexKind) -> bool {
        if self.entries.contains_key(&id) {
            return false;
        }
        self.entries
            .insert(id, CondEntry::new(mutex_id, mutex_kind));
        true
    }

    /// Destroy a cond. Returns the removed entry (for diagnostic
    /// inspection) or `None` if the id was unknown. Destroy while
    /// waiters are parked is a guest programming error; this method
    /// does not enforce that -- the handler above validates.
    pub fn destroy(&mut self, id: u32) -> Option<CondEntry> {
        self.entries.remove(&id)
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

    /// Enqueue `waiter` on the waiter list for `id`. Returns `true`
    /// on success, `false` if `id` is unknown or `waiter` is already
    /// parked on this cond.
    pub fn enqueue_waiter(&mut self, id: u32, waiter: PpuThreadId) -> bool {
        let Some(entry) = self.entries.get_mut(&id) else {
            return false;
        };
        entry.waiters.enqueue(waiter)
    }

    /// Pop the head of the waiter list for `id`. Returns the thread
    /// that should be woken, or `None` if the cond is unknown or has
    /// no waiters (non-sticky: no pending-signal counter is
    /// maintained). Used by `sys_cond_signal`.
    pub fn signal_one(&mut self, id: u32) -> Option<PpuThreadId> {
        let entry = self.entries.get_mut(&id)?;
        entry.waiters.dequeue_one()
    }

    /// Drain all waiters in FIFO order. Returns an empty vec if the
    /// cond is unknown or has no waiters. Used by
    /// `sys_cond_signal_all`.
    pub fn signal_all(&mut self, id: u32) -> Vec<PpuThreadId> {
        let Some(entry) = self.entries.get_mut(&id) else {
            return Vec::new();
        };
        entry.waiters.drain_all().collect()
    }

    /// Remove `target` from the cond's waiter list if present.
    /// Returns `true` if `target` was parked on this cond. Used by
    /// `sys_cond_signal_to`.
    pub fn signal_to(&mut self, id: u32, target: PpuThreadId) -> bool {
        let Some(entry) = self.entries.get_mut(&id) else {
            return false;
        };
        entry.waiters.remove(target)
    }

    /// FNV-1a digest of the table for state-hash folding. Walks
    /// entries in BTreeMap (ascending id) order; within each entry
    /// folds associated mutex id, mutex-kind discriminant, and the
    /// waiter list in enqueue order.
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
        assert!(t.create_with_id(0x4000_0010, 7, CondMutexKind::LwMutex));
        let e = t.lookup(0x4000_0010).unwrap();
        assert_eq!(e.mutex_id(), 7);
        assert_eq!(e.mutex_kind(), CondMutexKind::LwMutex);
        assert!(e.waiters().is_empty());
    }

    #[test]
    fn create_with_id_rejects_duplicate() {
        let mut t = CondTable::new();
        assert!(t.create_with_id(5, 1, CondMutexKind::LwMutex));
        assert!(!t.create_with_id(5, 2, CondMutexKind::Mutex));
        // First binding preserved.
        let e = t.lookup(5).unwrap();
        assert_eq!(e.mutex_id(), 1);
        assert_eq!(e.mutex_kind(), CondMutexKind::LwMutex);
    }

    #[test]
    fn destroy_removes_entry() {
        let mut t = CondTable::new();
        t.create_with_id(5, 1, CondMutexKind::LwMutex);
        assert!(t.destroy(5).is_some());
        assert!(t.lookup(5).is_none());
        assert!(t.destroy(5).is_none());
    }

    #[test]
    fn enqueue_waiter_fifo_order() {
        let mut t = CondTable::new();
        t.create_with_id(5, 1, CondMutexKind::LwMutex);
        assert!(t.enqueue_waiter(5, tid(0x0100_0001)));
        assert!(t.enqueue_waiter(5, tid(0x0100_0002)));
        assert!(t.enqueue_waiter(5, tid(0x0100_0003)));
        let seen: Vec<_> = t.lookup(5).unwrap().waiters().iter().collect();
        assert_eq!(
            seen,
            vec![tid(0x0100_0001), tid(0x0100_0002), tid(0x0100_0003)],
        );
    }

    #[test]
    fn enqueue_waiter_unknown_id_returns_false() {
        let mut t = CondTable::new();
        assert!(!t.enqueue_waiter(99, tid(0x0100_0001)));
    }

    #[test]
    fn enqueue_waiter_duplicate_rejected() {
        let mut t = CondTable::new();
        t.create_with_id(5, 1, CondMutexKind::LwMutex);
        assert!(t.enqueue_waiter(5, tid(0x0100_0001)));
        assert!(!t.enqueue_waiter(5, tid(0x0100_0001)));
    }

    #[test]
    fn signal_one_pops_head() {
        let mut t = CondTable::new();
        t.create_with_id(5, 1, CondMutexKind::LwMutex);
        t.enqueue_waiter(5, tid(0x0100_0001));
        t.enqueue_waiter(5, tid(0x0100_0002));
        t.enqueue_waiter(5, tid(0x0100_0003));
        assert_eq!(t.signal_one(5), Some(tid(0x0100_0001)));
        assert_eq!(t.signal_one(5), Some(tid(0x0100_0002)));
        assert_eq!(t.signal_one(5), Some(tid(0x0100_0003)));
        assert_eq!(t.signal_one(5), None);
    }

    #[test]
    fn signal_one_with_no_waiters_is_lost() {
        // Non-sticky: a signal with no waiters does not produce a
        // "pending" state. A subsequent enqueue must still block.
        let mut t = CondTable::new();
        t.create_with_id(5, 1, CondMutexKind::LwMutex);
        assert_eq!(t.signal_one(5), None);
        assert_eq!(t.signal_one(5), None);
        // Enqueue after the lost signals: the waiter is still parked,
        // not implicitly released.
        assert!(t.enqueue_waiter(5, tid(0x0100_0001)));
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
        t.create_with_id(5, 1, CondMutexKind::LwMutex);
        t.enqueue_waiter(5, tid(0x0100_0001));
        t.enqueue_waiter(5, tid(0x0100_0002));
        t.enqueue_waiter(5, tid(0x0100_0003));
        let woken = t.signal_all(5);
        assert_eq!(
            woken,
            vec![tid(0x0100_0001), tid(0x0100_0002), tid(0x0100_0003)],
        );
        assert!(t.lookup(5).unwrap().waiters().is_empty());
    }

    #[test]
    fn signal_all_with_no_waiters_returns_empty() {
        let mut t = CondTable::new();
        t.create_with_id(5, 1, CondMutexKind::LwMutex);
        assert!(t.signal_all(5).is_empty());
    }

    #[test]
    fn signal_all_unknown_id_returns_empty() {
        let mut t = CondTable::new();
        assert!(t.signal_all(99).is_empty());
    }

    #[test]
    fn signal_to_removes_specific_waiter() {
        let mut t = CondTable::new();
        t.create_with_id(5, 1, CondMutexKind::LwMutex);
        t.enqueue_waiter(5, tid(0x0100_0001));
        t.enqueue_waiter(5, tid(0x0100_0002));
        t.enqueue_waiter(5, tid(0x0100_0003));
        assert!(t.signal_to(5, tid(0x0100_0002)));
        let remaining: Vec<_> = t.lookup(5).unwrap().waiters().iter().collect();
        assert_eq!(remaining, vec![tid(0x0100_0001), tid(0x0100_0003)]);
    }

    #[test]
    fn signal_to_missing_target_returns_false() {
        let mut t = CondTable::new();
        t.create_with_id(5, 1, CondMutexKind::LwMutex);
        t.enqueue_waiter(5, tid(0x0100_0001));
        assert!(!t.signal_to(5, tid(0x0100_0099)));
    }

    #[test]
    fn signal_to_unknown_id_returns_false() {
        let mut t = CondTable::new();
        assert!(!t.signal_to(99, tid(0x0100_0001)));
    }

    #[test]
    fn heavy_mutex_binding_is_preserved() {
        let mut t = CondTable::new();
        t.create_with_id(0x4000_0020, 0x4000_0005, CondMutexKind::Mutex);
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
        a.create_with_id(5, 1, CondMutexKind::LwMutex);
        b.create_with_id(5, 1, CondMutexKind::Mutex);
        assert_ne!(a.state_hash(), b.state_hash());
    }

    #[test]
    fn state_hash_distinguishes_mutex_id() {
        let mut a = CondTable::new();
        let mut b = CondTable::new();
        a.create_with_id(5, 1, CondMutexKind::LwMutex);
        b.create_with_id(5, 2, CondMutexKind::LwMutex);
        assert_ne!(a.state_hash(), b.state_hash());
    }

    #[test]
    fn state_hash_distinguishes_waiter_order() {
        let mut a = CondTable::new();
        let mut b = CondTable::new();
        a.create_with_id(5, 1, CondMutexKind::LwMutex);
        b.create_with_id(5, 1, CondMutexKind::LwMutex);
        a.enqueue_waiter(5, tid(0x0100_0001));
        a.enqueue_waiter(5, tid(0x0100_0002));
        b.enqueue_waiter(5, tid(0x0100_0002));
        b.enqueue_waiter(5, tid(0x0100_0001));
        assert_ne!(a.state_hash(), b.state_hash());
    }

    #[test]
    fn state_hash_ignores_ephemeral_signal_attempts() {
        // A lost signal (no waiter present) leaves the table
        // unchanged; the hash must match a table that never saw the
        // signal. This anchors the non-sticky contract at the hash
        // level.
        let mut a = CondTable::new();
        let mut b = CondTable::new();
        a.create_with_id(5, 1, CondMutexKind::LwMutex);
        b.create_with_id(5, 1, CondMutexKind::LwMutex);
        a.signal_one(5);
        a.signal_all(5);
        a.signal_to(5, tid(0x0100_0099));
        assert_eq!(a.state_hash(), b.state_hash());
    }
}
