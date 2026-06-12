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
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum MutexCreateError {
    /// An entry with this id was already present.
    #[error("mutex create: {0}")]
    IdCollision(#[source] super::IdCollision),
}

/// Failure modes of [`MutexTable::enqueue_waiter`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum MutexEnqueueError {
    /// No mutex with this id.
    #[error("mutex enqueue: unknown id")]
    UnknownId,
    /// Thread is already on the waiter list. Always state
    /// corruption; callers route to `record_invariant_break`.
    #[error("mutex enqueue: duplicate waiter")]
    DuplicateWaiter,
    /// Thread is the current owner. Reachable from guest
    /// recursive-lock attempts.
    #[error("mutex enqueue: waiter is owner")]
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
            return Err(MutexCreateError::IdCollision(super::IdCollision { id }));
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
#[path = "tests/mutex_tests.rs"]
mod tests;
