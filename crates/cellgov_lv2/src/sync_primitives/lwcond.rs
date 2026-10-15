//! Lightweight condition variable table.
//!
//! The kernel side of an lwcond is the id of the lwmutex it is bound
//! to and a FIFO queue of waiters. The release of the lwmutex at wait
//! and its hand-off at signal live in the dispatch layer.

use crate::ppu_thread::PpuThreadId;
use crate::sync_primitives::WaiterList;
use cellgov_mem::lanes::{source, LaneMap, LaneValue, ObjectLanes};

/// A single lightweight condition variable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LwCondEntry {
    lwmutex_id: u32,
    waiters: WaiterList,
}

impl LwCondEntry {
    /// Kernel id of the lwmutex the create bound the lwcond to.
    pub fn lwmutex_id(&self) -> u32 {
        self.lwmutex_id
    }

    /// Read-only view of the waiter list.
    pub fn waiters(&self) -> &WaiterList {
        &self.waiters
    }
}

impl LaneValue for LwCondEntry {
    fn lanes(&self, lanes: &mut ObjectLanes) {
        lanes.lane(1, 0, u64::from(self.lwmutex_id));
        self.waiters.push_lanes(lanes, 2);
    }
}

/// Table of lightweight condition variables.
#[derive(Debug, Clone)]
pub struct LwCondTable {
    entries: LaneMap<u32, LwCondEntry>,
}

impl Default for LwCondTable {
    fn default() -> Self {
        Self {
            entries: LaneMap::new(source::LWCOND, u64::from),
        }
    }
}

impl LwCondTable {
    /// Construct an empty table.
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert a fresh entry bound to `lwmutex_id` under `id`.
    pub fn create_with_id(&mut self, id: u32, lwmutex_id: u32) {
        debug_assert!(
            !self.entries.contains_key(id),
            "lwcond {id:#x} registered twice"
        );
        self.entries.insert(
            id,
            LwCondEntry {
                lwmutex_id,
                waiters: WaiterList::new(),
            },
        );
    }

    /// Remove the entry; `None` if the id was unknown.
    ///
    /// # Panics
    ///
    /// In a debug build, panics if the entry has waiters. The caller
    /// rejects such an entry first.
    pub fn destroy(&mut self, id: u32) -> Option<LwCondEntry> {
        let entry = self.entries.remove(id)?;
        debug_assert!(
            entry.waiters.is_empty(),
            "lwcond {:#x} destroyed with {} parked waiter(s)",
            id,
            entry.waiters.len(),
        );
        Some(entry)
    }

    /// Read-only lookup.
    pub fn lookup(&self, id: u32) -> Option<&LwCondEntry> {
        self.entries.get(id)
    }

    /// Number of tracked lwconds.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the table has no entries.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Park `waiter` at the tail of the waiter list.
    ///
    /// Returns `false` in these cases:
    ///
    /// - The id is unknown.
    /// - The thread is already parked here.
    pub fn enqueue_waiter(&mut self, id: u32, waiter: PpuThreadId) -> bool {
        let Some(mut entry) = self.entries.get_mut(id) else {
            return false;
        };
        entry.waiters.enqueue(waiter).is_ok()
    }

    /// Pop the head of the waiter list.
    ///
    /// Returns `None` in these cases:
    ///
    /// - The id is unknown.
    /// - The waiter list is empty.
    pub fn dequeue_one(&mut self, id: u32) -> Option<PpuThreadId> {
        self.entries.get_mut(id)?.waiters.dequeue_one()
    }

    /// Drain the waiter list in FIFO order; `None` if the id is unknown.
    pub fn drain_all(&mut self, id: u32) -> Option<Vec<PpuThreadId>> {
        let mut entry = self.entries.get_mut(id)?;
        Some(entry.waiters.drain_all().collect())
    }

    /// Remove `waiter` from the waiter list.
    ///
    /// Returns `false` in these cases:
    ///
    /// - The id is unknown.
    /// - The thread is not parked here.
    pub fn remove_waiter(&mut self, id: u32, waiter: PpuThreadId) -> bool {
        let Some(mut entry) = self.entries.get_mut(id) else {
            return false;
        };
        entry.waiters.remove(waiter)
    }

    /// Remove every waiter in `threads` from every lwcond, for a process exit.
    ///
    /// The other waiters keep their order. The call returns the
    /// `(id, thread)` pairs in table order.
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

    /// The table's partial of the sync-state sum.
    pub fn sync_partial(&self) -> u128 {
        self.entries.partial()
    }

    /// [`Self::sync_partial`] computed from every entry.
    pub fn sync_partial_from_scratch(&self) -> u128 {
        self.entries.partial_from_scratch()
    }
}
