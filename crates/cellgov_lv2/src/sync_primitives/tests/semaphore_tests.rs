//! Semaphore table tests -- count bounds, try-wait decrement, and post-and-wake FIFO order.

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
        Err(SemaphoreCreateError::IdCollision(
            crate::sync_primitives::IdCollision { id: 5 }
        )),
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
