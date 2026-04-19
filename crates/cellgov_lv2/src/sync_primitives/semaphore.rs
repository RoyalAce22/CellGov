//! Counting semaphore table.
//!
//! Owns the state for `sys_semaphore_create` / `_destroy` /
//! `_wait` / `_post` / `_trywait` / `_get_value`. Each entry is
//! keyed by a guest-visible `u32` id minted by the host's shared
//! kernel-object allocator and carries:
//!
//!   * `count: i32` -- current available resource count. Signed to
//!     match the ABI's `sys_semaphore_value_t`. Negative values are
//!     possible if a post arrives after callers passed the check
//!     in a permissive implementation, but CellGov's semaphore
//!     never goes negative because wait atomically decrements
//!     with the runtime's serialized dispatch.
//!   * `max: i32` -- upper bound on `count`. Posts beyond the max
//!     return EINVAL without waking waiters.
//!   * `waiters: WaiterList` -- FIFO queue of PPU threads parked
//!     on `_wait` after observing count == 0.
//!
//! The "wake one, do not increment" rule on post is what makes a
//! counting semaphore FIFO-fair: a post with at least one parked
//! waiter hands the resource slot directly to that waiter rather
//! than incrementing and letting a later caller race past. This
//! is the observable-contract difference from an event-counter;
//! getting it wrong produces lost wakeups under contention.

use crate::ppu_thread::PpuThreadId;
use crate::sync_primitives::WaiterList;
use std::collections::BTreeMap;

/// Outcome of a `try_wait` call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SemaphoreWait {
    /// The caller consumed a slot (count was > 0, now decremented).
    /// Syscall returns `CELL_OK` without parking.
    Acquired,
    /// The caller found count == 0. The syscall should park the
    /// caller via `enqueue_waiter` (for `_wait`) or return EBUSY
    /// (for `_trywait`).
    Empty,
}

/// Outcome of a `post_and_wake` call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SemaphorePost {
    /// A parked waiter received the posted slot. Runtime should
    /// wake `new_owner` with r3 = 0. Count was NOT incremented --
    /// the waiter consumed the post directly.
    Woke {
        /// Thread that consumed the post.
        new_owner: PpuThreadId,
    },
    /// No waiters were parked; count was incremented by 1.
    Incremented,
    /// Post would push count past the semaphore's max. The table
    /// is unchanged. Syscall returns EINVAL.
    OverMax,
    /// The semaphore id is unknown. Syscall returns ESRCH.
    Unknown,
}

/// A single counting semaphore.
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

    /// Upper bound on `count` set at create time.
    pub fn max(&self) -> i32 {
        self.max
    }

    /// Read-only view of the waiter list.
    pub fn waiters(&self) -> &WaiterList {
        &self.waiters
    }
}

/// Table of counting semaphores. Ids come from the host's shared
/// kernel-object allocator; `create_with_id` is the only entry
/// point.
#[derive(Debug, Clone, Default)]
pub struct SemaphoreTable {
    entries: BTreeMap<u32, SemaphoreEntry>,
}

impl SemaphoreTable {
    /// Construct an empty table.
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert a fresh entry with the given id, initial count, and
    /// max. Returns `false` if `id` is already present (allocator
    /// collision) or if `initial > max` (guest error -- caller
    /// should surface EINVAL).
    pub fn create_with_id(&mut self, id: u32, initial: i32, max: i32) -> bool {
        if self.entries.contains_key(&id) {
            return false;
        }
        if initial > max || initial < 0 || max < 0 {
            return false;
        }
        self.entries.insert(id, SemaphoreEntry::new(initial, max));
        true
    }

    /// Destroy a semaphore. Returns the removed entry (for
    /// diagnostic inspection) or `None` if the id was unknown.
    pub fn destroy(&mut self, id: u32) -> Option<SemaphoreEntry> {
        self.entries.remove(&id)
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

    /// Try to consume a slot on `id`.
    ///
    /// Returns `None` if `id` is unknown. Otherwise returns
    /// `Some(Acquired)` and decrements `count` when count > 0, or
    /// `Some(Empty)` without mutation when count == 0. The caller
    /// parks via `enqueue_waiter` for `_wait` or returns EBUSY for
    /// `_trywait`.
    pub fn try_wait(&mut self, id: u32) -> Option<SemaphoreWait> {
        let entry = self.entries.get_mut(&id)?;
        if entry.count > 0 {
            entry.count -= 1;
            Some(SemaphoreWait::Acquired)
        } else {
            Some(SemaphoreWait::Empty)
        }
    }

    /// Enqueue `waiter` on the waiter list for `id`. Returns
    /// `true` on success, `false` if `id` is unknown or `waiter`
    /// is already parked.
    pub fn enqueue_waiter(&mut self, id: u32, waiter: PpuThreadId) -> bool {
        let Some(entry) = self.entries.get_mut(&id) else {
            return false;
        };
        entry.waiters.enqueue(waiter)
    }

    /// Post one slot to `id`.
    ///
    /// Wake-or-increment semantics: if any waiter is parked, the
    /// head of the waiter list is woken and the count is NOT
    /// incremented (the waiter consumed the post). Otherwise count
    /// is incremented by 1. Returns `OverMax` (and does not
    /// increment) if the post would push count past `max`.
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
                    SemaphorePost::Incremented
                }
            }
        }
    }

    /// FNV-1a digest of the table for state-hash folding. Walks
    /// entries in BTreeMap order; within each entry folds count,
    /// max, and the waiter list in enqueue order. Tables differing
    /// in any of those produce distinct hashes.
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
    fn create_with_id_rejects_collision() {
        let mut t = SemaphoreTable::new();
        assert!(t.create_with_id(5, 0, 10));
        assert!(!t.create_with_id(5, 0, 10));
    }

    #[test]
    fn create_rejects_initial_above_max() {
        let mut t = SemaphoreTable::new();
        assert!(!t.create_with_id(5, 11, 10));
        assert!(t.lookup(5).is_none());
    }

    #[test]
    fn create_rejects_negative_initial_or_max() {
        let mut t = SemaphoreTable::new();
        assert!(!t.create_with_id(5, -1, 10));
        assert!(!t.create_with_id(5, 0, -1));
    }

    #[test]
    fn try_wait_with_positive_count_decrements() {
        let mut t = SemaphoreTable::new();
        t.create_with_id(1, 3, 10);
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
        t.create_with_id(1, 0, 10);
        assert_eq!(t.try_wait(1), Some(SemaphoreWait::Empty));
        assert_eq!(t.lookup(1).unwrap().count(), 0);
    }

    #[test]
    fn try_wait_unknown_id_is_none() {
        let mut t = SemaphoreTable::new();
        assert!(t.try_wait(99).is_none());
    }

    #[test]
    fn post_with_no_waiters_increments() {
        let mut t = SemaphoreTable::new();
        t.create_with_id(1, 0, 10);
        assert_eq!(t.post_and_wake(1), SemaphorePost::Incremented);
        assert_eq!(t.lookup(1).unwrap().count(), 1);
    }

    #[test]
    fn post_with_one_waiter_wakes_that_waiter_and_does_not_increment() {
        // The FIFO-fair semantics: a post with a waiter hands the
        // resource slot directly to the waiter, count stays at 0.
        let mut t = SemaphoreTable::new();
        t.create_with_id(1, 0, 10);
        t.enqueue_waiter(1, tid(0x0100_0001));
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
        t.create_with_id(1, 0, 10);
        t.enqueue_waiter(1, tid(0x0100_0001));
        t.enqueue_waiter(1, tid(0x0100_0002));
        t.enqueue_waiter(1, tid(0x0100_0003));
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
        // After all waiters are drained, next post increments.
        assert_eq!(t.post_and_wake(1), SemaphorePost::Incremented);
        assert_eq!(t.lookup(1).unwrap().count(), 1);
    }

    #[test]
    fn post_past_max_with_no_waiters_returns_over_max() {
        let mut t = SemaphoreTable::new();
        t.create_with_id(1, 5, 5);
        assert_eq!(t.post_and_wake(1), SemaphorePost::OverMax);
        // Count unchanged.
        assert_eq!(t.lookup(1).unwrap().count(), 5);
    }

    #[test]
    fn post_at_max_with_waiter_still_wakes_without_incrementing() {
        // The waiter-wake branch bypasses the max check because
        // the post is consumed by the waiter, not added to the
        // count. This is classical semaphore semantics.
        let mut t = SemaphoreTable::new();
        t.create_with_id(1, 5, 5);
        t.enqueue_waiter(1, tid(0x0100_0001));
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
    fn state_hash_empty_is_stable() {
        let a = SemaphoreTable::new();
        let b = SemaphoreTable::new();
        assert_eq!(a.state_hash(), b.state_hash());
    }

    #[test]
    fn state_hash_distinguishes_count() {
        let mut a = SemaphoreTable::new();
        let mut b = SemaphoreTable::new();
        a.create_with_id(1, 3, 10);
        b.create_with_id(1, 4, 10);
        assert_ne!(a.state_hash(), b.state_hash());
    }

    #[test]
    fn state_hash_distinguishes_waiter_order() {
        let mut a = SemaphoreTable::new();
        let mut b = SemaphoreTable::new();
        a.create_with_id(1, 0, 10);
        b.create_with_id(1, 0, 10);
        a.enqueue_waiter(1, tid(0x0100_0001));
        a.enqueue_waiter(1, tid(0x0100_0002));
        b.enqueue_waiter(1, tid(0x0100_0002));
        b.enqueue_waiter(1, tid(0x0100_0001));
        assert_ne!(a.state_hash(), b.state_hash());
    }
}
