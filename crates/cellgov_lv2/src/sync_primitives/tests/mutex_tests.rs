//! Mutex table tests -- ownership transfer, contention parking, and would-deadlock detection.

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
        Err(MutexCreateError::IdCollision(
            crate::sync_primitives::IdCollision { id: 5 }
        )),
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
