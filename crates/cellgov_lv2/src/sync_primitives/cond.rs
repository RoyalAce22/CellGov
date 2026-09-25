//! Condition variable table.
//!
//! Signals are non-sticky: `_signal` with no parked waiter is
//! lost. The two-hop release/re-acquire of the associated mutex
//! lives in the dispatch layer; this table only parks and
//! dequeues cond waiters.

use crate::dispatch::CondMutexKind;
use crate::ppu_thread::PpuThreadId;
use crate::sync_primitives::WaiterList;
use cellgov_mem::lanes::{source, LaneMap, LaneValue, ObjectLanes};

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

    /// Which mutex table holds the associated mutex.
    ///
    /// Heavy and lightweight mutex tables have distinct id
    /// spaces; the wake-side re-acquire needs this to route.
    pub fn mutex_kind(&self) -> CondMutexKind {
        self.mutex_kind
    }

    /// Read-only view of the waiter list.
    pub fn waiters(&self) -> &WaiterList {
        &self.waiters
    }
}

/// Failure modes of [`CondTable::create_with_id`].
///
/// Both variants indicate a dispatch-layer bug and fire a
/// `debug_assert!`. In release the existing entry is preserved
/// and the caller's binding is dropped.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum CondCreateError {
    /// Same id, same binding.
    #[error("cond create: redundant registration")]
    RedundantRegistration,
    /// Same id, different binding.
    #[error("cond create: {collision} (existing bound to mutex 0x{existing_mutex_id:08x} kind {existing_mutex_kind:?})")]
    IdCollision {
        /// Underlying id-collision diagnostic shared across primitives.
        #[source]
        collision: super::IdCollision,
        /// `mutex_id` of the pre-existing entry.
        existing_mutex_id: u32,
        /// `mutex_kind` of the pre-existing entry.
        existing_mutex_kind: CondMutexKind,
    },
}

/// Failure modes of [`CondTable::enqueue_waiter`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum CondEnqueueError {
    /// No cond with this id.
    #[error("cond enqueue: unknown id")]
    UnknownId,
    /// Thread is already parked on this cond; dispatch-layer bug
    /// (fires `debug_assert!`).
    #[error("cond enqueue: duplicate waiter")]
    DuplicateWaiter,
}

/// Failure modes of [`CondTable::signal_to`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum CondSignalToError {
    /// No cond with this id.
    #[error("cond signal-to: unknown id")]
    UnknownId,
    /// Cond exists but `target` is not on the waiter list.
    #[error("cond signal-to: target not waiting")]
    TargetNotWaiting,
}

/// Table of condition variables.
#[derive(Debug, Clone)]
pub struct CondTable {
    entries: LaneMap<u32, CondEntry>,
}

impl Default for CondTable {
    fn default() -> Self {
        Self {
            entries: LaneMap::new(source::COND, u64::from),
        }
    }
}

impl LaneValue for CondEntry {
    fn lanes(&self, lanes: &mut ObjectLanes) {
        lanes.lane(1, 0, u64::from(self.mutex_id));
        lanes.lane(2, 0, self.mutex_kind as u64 + 1);
        self.waiters.push_lanes(lanes, 3);
    }
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
        if let Some(existing) = self.entries.get(id) {
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
                collision: super::IdCollision { id },
                existing_mutex_id: existing.mutex_id,
                existing_mutex_kind: existing.mutex_kind,
            });
        }
        self.entries
            .insert(id, CondEntry::new(mutex_id, mutex_kind));
        Ok(())
    }

    /// Remove the entry; `None` if the id was unknown.
    ///
    /// Caller contract: reject non-empty-waiters before calling
    /// (`debug_assert!` fires on violation). If bypassed in
    /// release, callers **must** drain `entry.waiters()` and wake
    /// each parked thread; skipping this strands them forever.
    pub fn destroy(&mut self, id: u32) -> Option<CondEntry> {
        let entry = self.entries.remove(id)?;
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
        self.entries.get(id)
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
        let mut entry = self
            .entries
            .get_mut(id)
            .ok_or(CondEnqueueError::UnknownId)?;
        if entry.waiters.enqueue(waiter).is_err() {
            debug_assert!(false, "duplicate enqueue of {:?} on cond {:#x}", waiter, id,);
            return Err(CondEnqueueError::DuplicateWaiter);
        }
        Ok(())
    }

    /// Pop the head of the waiter list. `None` if the cond is
    /// unknown or empty.
    pub fn signal_one(&mut self, id: u32) -> Option<PpuThreadId> {
        let mut entry = self.entries.get_mut(id)?;
        entry.waiters.dequeue_one()
    }

    /// Drain all waiters in FIFO order.
    ///
    /// `None` vs `Some(empty)` distinguishes unknown-id from
    /// present-but-empty; the ABI surfaces different errors for
    /// each.
    pub fn signal_all(&mut self, id: u32) -> Option<Vec<PpuThreadId>> {
        let mut entry = self.entries.get_mut(id)?;
        Some(entry.waiters.drain_all().collect())
    }

    /// Remove every waiter in `threads` from every cond, preserving
    /// the order of survivors; returns `(id, thread)` pairs in table
    /// order. Process-exit purge.
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

    /// Remove `target` from the waiter list.
    pub fn signal_to(&mut self, id: u32, target: PpuThreadId) -> Result<(), CondSignalToError> {
        let mut entry = self
            .entries
            .get_mut(id)
            .ok_or(CondSignalToError::UnknownId)?;
        if entry.waiters.remove(target) {
            Ok(())
        } else {
            Err(CondSignalToError::TargetNotWaiting)
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
#[path = "tests/cond_tests.rs"]
mod tests;
