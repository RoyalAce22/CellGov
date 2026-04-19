//! Heavy mutex table.
//!
//! Owns the state for `sys_mutex_create` / `_destroy` / `_lock` /
//! `_unlock` / `_trylock`. Each entry is keyed by a guest-visible
//! `u32` id, allocated by the host's shared kernel-object
//! allocator (not the lwmutex-private allocator -- heavy mutex and
//! lwmutex live in distinct id spaces).
//!
//! Semantics mirror `LwMutexTable`: the table tracks the current
//! owner (if any), a FIFO waiter list, and the attribute bag
//! captured at create time. Only the blocking / waking contract is
//! load-bearing; attributes (recursion, priority policy, name)
//! are stored-and-ignored until a title exercises them.
//!
//! The heavy mutex and the lightweight mutex are kept as separate
//! tables, not a shared implementation. Their id spaces are
//! distinct at the ABI level (`sys_lwmutex_t` and `sys_mutex_t`
//! are different guest types), and the cond-wake re-acquire path
//! needs to distinguish between them via `CondMutexKind`.

use crate::ppu_thread::PpuThreadId;
use crate::sync_primitives::WaiterList;
use std::collections::BTreeMap;

/// Outcome of a `try_acquire` call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MutexAcquire {
    /// The caller became the owner. The caller's syscall returns
    /// `CELL_OK` and does not park.
    Acquired,
    /// The mutex is owned by another thread. The caller should
    /// either park (for `_lock`) or return EBUSY (for `_trylock`).
    Contended,
}

/// Outcome of a `release_and_wake_next` call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MutexRelease {
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

/// Attribute bag captured from `sys_mutex_create`.
///
/// Only `priority_policy` is surfaced; `recursive` and `name` are
/// stored-and-ignored as anti-churn. The PS3 ABI encodes protocol
/// / recursive as bit flags in the attr struct (see
/// `sys/mutex.h`); the handler decodes them into this structured
/// form before calling `MutexTable::create_with_attrs`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MutexAttrs {
    /// Priority-ordering policy (FIFO vs priority). Captured for
    /// diagnostics; the waiter list is strictly FIFO.
    pub priority_policy: u32,
    /// Whether the mutex is recursive (same thread can re-acquire).
    /// Stored-and-ignored: re-acquire is treated as Contended.
    pub recursive: bool,
    /// Raw protocol bits (LV2's `SYS_SYNC_FIFO` / `SYS_SYNC_PRIORITY`
    /// / `SYS_SYNC_PRIORITY_INHERIT`). Captured for diagnostics.
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

    /// Current owner (if any). `None` means the mutex is free.
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

/// Table of heavy mutexes. Ids come from the host's shared kernel
/// object allocator; `create_with_id` is the only entry point.
#[derive(Debug, Clone, Default)]
pub struct MutexTable {
    entries: BTreeMap<u32, MutexEntry>,
}

impl MutexTable {
    /// Construct an empty table.
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert a fresh entry with the given id and attributes. The
    /// caller (host) mints the id; this table stores the entry.
    /// Returns `false` if `id` is already present (collision -- a
    /// bug in the allocator, not a guest-visible error).
    pub fn create_with_id(&mut self, id: u32, attrs: MutexAttrs) -> bool {
        if self.entries.contains_key(&id) {
            return false;
        }
        self.entries.insert(id, MutexEntry::new(attrs));
        true
    }

    /// Destroy a mutex. Returns the removed entry (for diagnostic
    /// inspection) or `None` if the id was unknown.
    pub fn destroy(&mut self, id: u32) -> Option<MutexEntry> {
        self.entries.remove(&id)
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

    /// Attempt to acquire `id` on behalf of `caller`.
    ///
    /// Returns `None` if `id` is unknown. Otherwise returns
    /// `Some(Acquired)` when the mutex was unowned (and sets the
    /// owner to `caller`), or `Some(Contended)` when it is owned by
    /// some other thread. Re-acquiring an already-held mutex by the
    /// same caller is rejected with `Contended` -- recursive
    /// locking is not supported.
    pub fn try_acquire(&mut self, id: u32, caller: PpuThreadId) -> Option<MutexAcquire> {
        let entry = self.entries.get_mut(&id)?;
        if entry.owner.is_none() {
            entry.owner = Some(caller);
            Some(MutexAcquire::Acquired)
        } else {
            Some(MutexAcquire::Contended)
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

    /// Release `id` on behalf of `caller`. See `MutexRelease`.
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

    /// FNV-1a digest of the table for state-hash folding. Walks
    /// entries in BTreeMap (ascending id) order; within each entry
    /// folds owner, waiter list (in enqueue order), and attribute
    /// bag. Tables differing in any of those produce distinct
    /// hashes.
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
    fn create_with_id_rejects_collision() {
        let mut t = MutexTable::new();
        assert!(t.create_with_id(5, default_attrs()));
        assert!(!t.create_with_id(5, default_attrs()));
        assert_eq!(t.len(), 1);
    }

    #[test]
    fn try_acquire_unowned_sets_owner() {
        let mut t = MutexTable::new();
        t.create_with_id(1, default_attrs());
        assert_eq!(
            t.try_acquire(1, tid(0x0100_0001)),
            Some(MutexAcquire::Acquired),
        );
        assert_eq!(t.lookup(1).unwrap().owner(), Some(tid(0x0100_0001)));
    }

    #[test]
    fn try_acquire_contended_does_not_change_owner() {
        let mut t = MutexTable::new();
        t.create_with_id(1, default_attrs());
        let a = tid(0x0100_0001);
        let b = tid(0x0100_0002);
        t.try_acquire(1, a);
        assert_eq!(t.try_acquire(1, b), Some(MutexAcquire::Contended));
        assert_eq!(t.lookup(1).unwrap().owner(), Some(a));
    }

    #[test]
    fn try_acquire_same_thread_twice_is_contended() {
        // Heavy mutex is not recursive. Re-acquire by the owner
        // returns Contended.
        let mut t = MutexTable::new();
        t.create_with_id(1, default_attrs());
        let a = tid(0x0100_0001);
        t.try_acquire(1, a);
        assert_eq!(t.try_acquire(1, a), Some(MutexAcquire::Contended));
    }

    #[test]
    fn enqueue_waiter_preserves_fifo_order() {
        let mut t = MutexTable::new();
        t.create_with_id(1, default_attrs());
        t.try_acquire(1, tid(0x0100_0001));
        assert!(t.enqueue_waiter(1, tid(0x0100_0002)));
        assert!(t.enqueue_waiter(1, tid(0x0100_0003)));
        let seen: Vec<_> = t.lookup(1).unwrap().waiters().iter().collect();
        assert_eq!(seen, vec![tid(0x0100_0002), tid(0x0100_0003)]);
    }

    #[test]
    fn release_without_waiters_frees() {
        let mut t = MutexTable::new();
        t.create_with_id(1, default_attrs());
        let a = tid(0x0100_0001);
        t.try_acquire(1, a);
        assert_eq!(t.release_and_wake_next(1, a), MutexRelease::Freed);
        assert_eq!(t.lookup(1).unwrap().owner(), None);
    }

    #[test]
    fn release_with_waiters_transfers_to_head() {
        let mut t = MutexTable::new();
        t.create_with_id(1, default_attrs());
        let owner = tid(0x0100_0001);
        let w1 = tid(0x0100_0002);
        let w2 = tid(0x0100_0003);
        t.try_acquire(1, owner);
        t.enqueue_waiter(1, w1);
        t.enqueue_waiter(1, w2);
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
        t.create_with_id(1, default_attrs());
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
        t.create_with_id(1, attrs);
        assert_eq!(t.lookup(1).unwrap().attrs(), attrs);
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
        );
        b.create_with_id(
            1,
            MutexAttrs {
                priority_policy: 2,
                ..Default::default()
            },
        );
        assert_ne!(a.state_hash(), b.state_hash());
    }

    #[test]
    fn state_hash_distinguishes_waiter_order() {
        let mut a = MutexTable::new();
        let mut b = MutexTable::new();
        a.create_with_id(1, default_attrs());
        b.create_with_id(1, default_attrs());
        let owner = tid(0x0100_0001);
        a.try_acquire(1, owner);
        b.try_acquire(1, owner);
        a.enqueue_waiter(1, tid(0x0100_0002));
        a.enqueue_waiter(1, tid(0x0100_0003));
        b.enqueue_waiter(1, tid(0x0100_0003));
        b.enqueue_waiter(1, tid(0x0100_0002));
        assert_ne!(a.state_hash(), b.state_hash());
    }
}
