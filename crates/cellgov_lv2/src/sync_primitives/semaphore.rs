//! Counting semaphore table.
//!
//! Wake-or-increment: a post with a parked waiter hands the slot
//! to the FIFO head rather than incrementing `count`. `max`
//! therefore bounds `count` only in the quiescent state; posts
//! consumed by waiters bypass the check.

use crate::ppu_thread::PpuThreadId;
use crate::sync_primitives::WaiterList;
use std::collections::BTreeMap;

/// Outcome of a `try_wait` call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SemaphoreWait {
    /// Slot consumed.
    Acquired,
    /// Count was 0.
    Empty,
}

/// Outcome of a `post_and_wake` call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SemaphorePost {
    /// Head waiter consumed the post; count not incremented.
    Woke {
        /// Thread that consumed the post.
        new_owner: PpuThreadId,
    },
    /// No waiters; count incremented by 1.
    Incremented,
    /// Post would push count past `max`; table unchanged.
    OverMax,
    /// Unknown id.
    Unknown,
}

/// Failure modes of [`SemaphoreTable::create_with_id`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SemaphoreCreateError {
    /// An entry with this id was already present; allocator bug
    /// (fires `debug_assert!`).
    IdCollision,
    /// `initial > max`, or either value was negative.
    InvalidBounds,
}

/// Failure modes of [`SemaphoreTable::enqueue_waiter`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SemaphoreEnqueueError {
    /// No semaphore with this id.
    UnknownId,
    /// Thread is already parked on this semaphore;
    /// dispatch-layer bug (fires `debug_assert!`).
    DuplicateWaiter,
}

/// A single counting semaphore.
///
/// Invariant: `0 <= count <= max` in the quiescent state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemaphoreEntry {
    count: i32,
    max: i32,
    waiters: WaiterList,
}

impl SemaphoreEntry {
    fn new(initial: i32, max: i32) -> Self {
        Self {
            count: initial,
            max,
            waiters: WaiterList::new(),
        }
    }

    /// Current resource count.
    pub fn count(&self) -> i32 {
        self.count
    }

    /// Upper bound on `count` captured at create time.
    pub fn max(&self) -> i32 {
        self.max
    }

    /// Read-only view of the waiter list.
    pub fn waiters(&self) -> &WaiterList {
        &self.waiters
    }
}

/// Table of counting semaphores.
#[derive(Debug, Clone, Default)]
pub struct SemaphoreTable {
    entries: BTreeMap<u32, SemaphoreEntry>,
}

impl SemaphoreTable {
    /// Construct an empty table.
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert a fresh entry. See [`SemaphoreCreateError`].
    pub fn create_with_id(
        &mut self,
        id: u32,
        initial: i32,
        max: i32,
    ) -> Result<(), SemaphoreCreateError> {
        if self.entries.contains_key(&id) {
            debug_assert!(
                false,
                "semaphore {:#x} already present at create_with_id",
                id,
            );
            return Err(SemaphoreCreateError::IdCollision);
        }
        if initial > max || initial < 0 || max < 0 {
            return Err(SemaphoreCreateError::InvalidBounds);
        }
        self.entries.insert(id, SemaphoreEntry::new(initial, max));
        Ok(())
    }

    /// Remove the entry; `None` if the id was unknown.
    ///
    /// Dispatch rejects non-empty destroy with `CELL_EBUSY`, so
    /// the `debug_assert!` here is defense-in-depth. If bypassed
    /// in release, callers **must** drain `entry.waiters()` and
    /// wake each parked thread.
    pub fn destroy(&mut self, id: u32) -> Option<SemaphoreEntry> {
        let entry = self.entries.remove(&id)?;
        debug_assert!(
            entry.waiters.is_empty(),
            "semaphore {:#x} destroyed with {} parked waiter(s)",
            id,
            entry.waiters.len(),
        );
        Some(entry)
    }

    /// Read-only lookup.
    pub fn lookup(&self, id: u32) -> Option<&SemaphoreEntry> {
        self.entries.get(&id)
    }

    /// Number of tracked semaphores.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the table has no entries.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Try to consume a slot; `None` if `id` is unknown.
    pub fn try_wait(&mut self, id: u32) -> Option<SemaphoreWait> {
        let entry = self.entries.get_mut(&id)?;
        if entry.count > 0 {
            entry.count -= 1;
            debug_assert!(
                entry.count >= 0,
                "semaphore {:#x} count went negative after try_wait: {}",
                id,
                entry.count,
            );
            Some(SemaphoreWait::Acquired)
        } else {
            Some(SemaphoreWait::Empty)
        }
    }

    /// Enqueue a waiter.
    pub fn enqueue_waiter(
        &mut self,
        id: u32,
        waiter: PpuThreadId,
    ) -> Result<(), SemaphoreEnqueueError> {
        let entry = self
            .entries
            .get_mut(&id)
            .ok_or(SemaphoreEnqueueError::UnknownId)?;
        if entry.waiters.enqueue(waiter).is_err() {
            debug_assert!(
                false,
                "duplicate enqueue of {:?} on semaphore {:#x}",
                waiter, id,
            );
            return Err(SemaphoreEnqueueError::DuplicateWaiter);
        }
        Ok(())
    }

    /// Post one slot. Wakes the FIFO head (without incrementing)
    /// or increments `count`; `OverMax` if the increment would
    /// exceed `max`.
    pub fn post_and_wake(&mut self, id: u32) -> SemaphorePost {
        let Some(entry) = self.entries.get_mut(&id) else {
            return SemaphorePost::Unknown;
        };
        match entry.waiters.dequeue_one() {
            Some(new_owner) => SemaphorePost::Woke { new_owner },
            None => {
                if entry.count >= entry.max {
                    SemaphorePost::OverMax
                } else {
                    entry.count += 1;
                    debug_assert!(
                        entry.count <= entry.max,
                        "semaphore {:#x} count past max after post: count={} max={}",
                        id,
                        entry.count,
                        entry.max,
                    );
                    SemaphorePost::Incremented
                }
            }
        }
    }

    /// FNV-1a digest of the table's state.
    pub fn state_hash(&self) -> u64 {
        let mut hasher = cellgov_mem::Fnv1aHasher::new();
        hasher.write(&(self.entries.len() as u64).to_le_bytes());
        for (id, entry) in &self.entries {
            hasher.write(&id.to_le_bytes());
            hasher.write(&entry.count.to_le_bytes());
            hasher.write(&entry.max.to_le_bytes());
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
        let t = SemaphoreTable::new();
        assert!(t.is_empty());
        assert_eq!(t.len(), 0);
    }

    #[test]
    fn create_rejects_initial_above_max() {
        let mut t = SemaphoreTable::new();
        assert_eq!(
            t.create_with_id(5, 11, 10),
            Err(SemaphoreCreateError::InvalidBounds),
        );
        assert!(t.lookup(5).is_none());
    }

    #[test]
    fn create_rejects_negative_initial_or_max() {
        let mut t = SemaphoreTable::new();
        assert_eq!(
            t.create_with_id(5, -1, 10),
            Err(SemaphoreCreateError::InvalidBounds),
        );
        assert_eq!(
            t.create_with_id(5, 0, -1),
            Err(SemaphoreCreateError::InvalidBounds),
        );
    }

    #[test]
    fn try_wait_with_positive_count_decrements() {
        let mut t = SemaphoreTable::new();
        t.create_with_id(1, 3, 10).unwrap();
        assert_eq!(t.try_wait(1), Some(SemaphoreWait::Acquired));
        assert_eq!(t.lookup(1).unwrap().count(), 2);
        assert_eq!(t.try_wait(1), Some(SemaphoreWait::Acquired));
        assert_eq!(t.lookup(1).unwrap().count(), 1);
        assert_eq!(t.try_wait(1), Some(SemaphoreWait::Acquired));
        assert_eq!(t.lookup(1).unwrap().count(), 0);
    }

    #[test]
    fn try_wait_with_zero_count_returns_empty_and_preserves_state() {
        let mut t = SemaphoreTable::new();
        t.create_with_id(1, 0, 10).unwrap();
        assert_eq!(t.try_wait(1), Some(SemaphoreWait::Empty));
        assert_eq!(t.lookup(1).unwrap().count(), 0);
    }

    #[test]
    fn try_wait_unknown_id_is_none() {
        let mut t = SemaphoreTable::new();
        assert!(t.try_wait(99).is_none());
    }

    #[test]
    fn try_wait_after_destroy_returns_none() {
        let mut t = SemaphoreTable::new();
        t.create_with_id(1, 3, 10).unwrap();
        t.destroy(1);
        assert!(t.try_wait(1).is_none());
    }

    #[test]
    fn post_with_no_waiters_increments() {
        let mut t = SemaphoreTable::new();
        t.create_with_id(1, 0, 10).unwrap();
        assert_eq!(t.post_and_wake(1), SemaphorePost::Incremented);
        assert_eq!(t.lookup(1).unwrap().count(), 1);
    }

    #[test]
    fn post_with_one_waiter_wakes_that_waiter_and_does_not_increment() {
        let mut t = SemaphoreTable::new();
        t.create_with_id(1, 0, 10).unwrap();
        t.enqueue_waiter(1, tid(0x0100_0001)).unwrap();
        assert_eq!(
            t.post_and_wake(1),
            SemaphorePost::Woke {
                new_owner: tid(0x0100_0001)
            },
        );
        assert_eq!(t.lookup(1).unwrap().count(), 0);
        assert!(t.lookup(1).unwrap().waiters().is_empty());
    }

    #[test]
    fn post_with_multiple_waiters_wakes_head_in_fifo_order() {
        let mut t = SemaphoreTable::new();
        t.create_with_id(1, 0, 10).unwrap();
        t.enqueue_waiter(1, tid(0x0100_0001)).unwrap();
        t.enqueue_waiter(1, tid(0x0100_0002)).unwrap();
        t.enqueue_waiter(1, tid(0x0100_0003)).unwrap();
        assert_eq!(
            t.post_and_wake(1),
            SemaphorePost::Woke {
                new_owner: tid(0x0100_0001)
            },
        );
        assert_eq!(
            t.post_and_wake(1),
            SemaphorePost::Woke {
                new_owner: tid(0x0100_0002)
            },
        );
        assert_eq!(
            t.post_and_wake(1),
            SemaphorePost::Woke {
                new_owner: tid(0x0100_0003)
            },
        );
        assert_eq!(t.post_and_wake(1), SemaphorePost::Incremented);
        assert_eq!(t.lookup(1).unwrap().count(), 1);
    }

    #[test]
    fn post_past_max_with_no_waiters_returns_over_max() {
        let mut t = SemaphoreTable::new();
        t.create_with_id(1, 5, 5).unwrap();
        assert_eq!(t.post_and_wake(1), SemaphorePost::OverMax);
        assert_eq!(t.lookup(1).unwrap().count(), 5);
    }

    #[test]
    fn post_at_max_with_waiter_still_wakes_without_incrementing() {
        let mut t = SemaphoreTable::new();
        t.create_with_id(1, 5, 5).unwrap();
        t.enqueue_waiter(1, tid(0x0100_0001)).unwrap();
        assert_eq!(
            t.post_and_wake(1),
            SemaphorePost::Woke {
                new_owner: tid(0x0100_0001)
            },
        );
        assert_eq!(t.lookup(1).unwrap().count(), 5);
    }

    #[test]
    fn post_unknown_id_is_unknown() {
        let mut t = SemaphoreTable::new();
        assert_eq!(t.post_and_wake(99), SemaphorePost::Unknown);
    }

    #[test]
    fn enqueue_waiter_unknown_id_returns_err() {
        let mut t = SemaphoreTable::new();
        assert_eq!(
            t.enqueue_waiter(99, tid(0x0100_0001)),
            Err(SemaphoreEnqueueError::UnknownId),
        );
    }

    #[test]
    fn state_hash_empty_is_stable() {
        let a = SemaphoreTable::new();
        let b = SemaphoreTable::new();
        assert_eq!(a.state_hash(), b.state_hash());
    }

    #[test]
    fn state_hash_distinguishes_count() {
        let mut a = SemaphoreTable::new();
        let mut b = SemaphoreTable::new();
        a.create_with_id(1, 3, 10).unwrap();
        b.create_with_id(1, 4, 10).unwrap();
        assert_ne!(a.state_hash(), b.state_hash());
    }

    #[test]
    fn state_hash_distinguishes_waiter_order() {
        let mut a = SemaphoreTable::new();
        let mut b = SemaphoreTable::new();
        a.create_with_id(1, 0, 10).unwrap();
        b.create_with_id(1, 0, 10).unwrap();
        a.enqueue_waiter(1, tid(0x0100_0001)).unwrap();
        a.enqueue_waiter(1, tid(0x0100_0002)).unwrap();
        b.enqueue_waiter(1, tid(0x0100_0002)).unwrap();
        b.enqueue_waiter(1, tid(0x0100_0001)).unwrap();
        assert_ne!(a.state_hash(), b.state_hash());
    }

    #[cfg(debug_assertions)]
    #[test]
    #[should_panic(expected = "already present")]
    fn create_with_id_collision_fires_debug_assert() {
        let mut t = SemaphoreTable::new();
        t.create_with_id(5, 0, 10).unwrap();
        let _ = t.create_with_id(5, 0, 10);
    }

    #[cfg(debug_assertions)]
    #[test]
    #[should_panic(expected = "duplicate enqueue")]
    fn duplicate_enqueue_fires_debug_assert() {
        let mut t = SemaphoreTable::new();
        t.create_with_id(1, 0, 10).unwrap();
        t.enqueue_waiter(1, tid(0x0100_0001)).unwrap();
        let _ = t.enqueue_waiter(1, tid(0x0100_0001));
    }

    #[cfg(debug_assertions)]
    #[test]
    #[should_panic(expected = "destroyed with")]
    fn destroy_with_parked_waiters_fires_debug_assert() {
        let mut t = SemaphoreTable::new();
        t.create_with_id(1, 0, 10).unwrap();
        t.enqueue_waiter(1, tid(0x0100_0001)).unwrap();
        let _ = t.destroy(1);
    }

    #[cfg(not(debug_assertions))]
    #[test]
    fn create_with_id_collision_returns_err_in_release() {
        let mut t = SemaphoreTable::new();
        t.create_with_id(5, 0, 10).unwrap();
        assert_eq!(
            t.create_with_id(5, 0, 10),
            Err(SemaphoreCreateError::IdCollision),
        );
        assert_eq!(t.len(), 1);
    }

    #[cfg(not(debug_assertions))]
    #[test]
    fn duplicate_enqueue_returns_err_in_release() {
        let mut t = SemaphoreTable::new();
        t.create_with_id(1, 0, 10).unwrap();
        t.enqueue_waiter(1, tid(0x0100_0001)).unwrap();
        assert_eq!(
            t.enqueue_waiter(1, tid(0x0100_0001)),
            Err(SemaphoreEnqueueError::DuplicateWaiter),
        );
        assert_eq!(t.lookup(1).unwrap().waiters().len(), 1);
    }

    #[cfg(not(debug_assertions))]
    #[test]
    fn destroy_with_parked_waiters_returns_entry_unchanged_in_release() {
        let mut t = SemaphoreTable::new();
        t.create_with_id(1, 0, 10).unwrap();
        let waiter = tid(0x0100_0001);
        t.enqueue_waiter(1, waiter).unwrap();
        let removed = t.destroy(1).unwrap();
        let parked: Vec<_> = removed.waiters().iter().collect();
        assert_eq!(parked, vec![waiter]);
        assert!(t.lookup(1).is_none());
    }
}
