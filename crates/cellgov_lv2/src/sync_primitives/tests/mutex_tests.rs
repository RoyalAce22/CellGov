//! Mutex table tests -- ownership transfer, contention parking, would-deadlock detection, and recursive relock counting.

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

fn recursive_attrs() -> MutexAttrs {
    MutexAttrs {
        recursive: true,
        ..Default::default()
    }
}

#[test]
fn a_non_recursive_relock_by_the_owner_stays_would_deadlock() {
    let mut t = MutexTable::new();
    t.create_with_id(1, default_attrs()).unwrap();
    let a = tid(0x0100_0001);
    t.acquire_or_enqueue(1, a);
    assert_eq!(
        t.acquire_or_enqueue(1, a),
        MutexAcquireOrEnqueue::WouldDeadlock,
    );
    assert_eq!(t.lookup(1).unwrap().lock_count(), 0);
}

#[test]
fn a_recursive_relock_by_the_owner_increments_instead_of_deadlocking() {
    let mut t = MutexTable::new();
    t.create_with_id(1, recursive_attrs()).unwrap();
    let a = tid(0x0100_0001);
    assert_eq!(t.acquire_or_enqueue(1, a), MutexAcquireOrEnqueue::Acquired);
    assert_eq!(t.acquire_or_enqueue(1, a), MutexAcquireOrEnqueue::Recursed);
    assert_eq!(t.acquire_or_enqueue(1, a), MutexAcquireOrEnqueue::Recursed);
    assert_eq!(t.lookup(1).unwrap().owner(), Some(a));
    assert_eq!(t.lookup(1).unwrap().lock_count(), 2);
    assert!(t.lookup(1).unwrap().waiters().is_empty());
}

#[test]
fn a_recursive_relock_needs_matching_unlocks_before_a_waiter_is_granted() {
    let mut t = MutexTable::new();
    t.create_with_id(1, recursive_attrs()).unwrap();
    let owner = tid(0x0100_0001);
    let waiter = tid(0x0100_0002);
    t.acquire_or_enqueue(1, owner);
    assert_eq!(
        t.acquire_or_enqueue(1, owner),
        MutexAcquireOrEnqueue::Recursed,
    );
    assert_eq!(
        t.acquire_or_enqueue(1, waiter),
        MutexAcquireOrEnqueue::Enqueued,
    );
    assert!(t.unlock_decrement(1, owner), "one recursive hold remains");
    assert_eq!(
        t.lookup(1).unwrap().owner(),
        Some(owner),
        "decrement above zero must not release ownership"
    );
    assert_eq!(t.lookup(1).unwrap().waiters().len(), 1);
    assert!(
        !t.unlock_decrement(1, owner),
        "count is back at zero; the final unlock is a release"
    );
    assert_eq!(
        t.release_and_wake_next(1, owner),
        MutexRelease::Transferred { new_owner: waiter },
    );
}

#[test]
fn unlock_decrement_without_a_recursive_hold_returns_false() {
    let mut t = MutexTable::new();
    assert!(!t.unlock_decrement(99, tid(0x0100_0001)), "unknown id");
    t.create_with_id(1, recursive_attrs()).unwrap();
    let owner = tid(0x0100_0001);
    let other = tid(0x0100_0002);
    assert!(!t.unlock_decrement(1, owner), "unowned mutex");
    t.acquire_or_enqueue(1, owner);
    assert!(!t.unlock_decrement(1, owner), "held once, count zero");
    t.acquire_or_enqueue(1, owner);
    assert!(!t.unlock_decrement(1, other), "non-owner never decrements");
    assert_eq!(t.lookup(1).unwrap().lock_count(), 1);
}

#[test]
fn a_saturated_lock_count_reports_count_saturated_and_does_not_wrap() {
    let mut t = MutexTable::new();
    t.create_with_id(1, recursive_attrs()).unwrap();
    let a = tid(0x0100_0001);
    t.acquire_or_enqueue(1, a);
    t.set_lock_count_for_test(1, u32::MAX);
    assert_eq!(
        t.acquire_or_enqueue(1, a),
        MutexAcquireOrEnqueue::CountSaturated,
    );
    assert_eq!(t.lookup(1).unwrap().lock_count(), u32::MAX);
    assert_eq!(t.lookup(1).unwrap().owner(), Some(a));
}

#[test]
fn a_full_release_clears_outstanding_recursive_holds() {
    // The cond-wait release path gives up the mutex outright; a
    // stale count must not survive into the next owner's hold.
    let mut t = MutexTable::new();
    t.create_with_id(1, recursive_attrs()).unwrap();
    let a = tid(0x0100_0001);
    t.acquire_or_enqueue(1, a);
    t.acquire_or_enqueue(1, a);
    assert_eq!(t.release_and_wake_next(1, a), MutexRelease::Freed);
    assert_eq!(t.lookup(1).unwrap().lock_count(), 0);
    assert_eq!(t.lookup(1).unwrap().owner(), None);
}

#[test]
fn a_release_that_drops_recursive_holds_is_witnessed() {
    let mut t = MutexTable::new();
    t.create_with_id(1, recursive_attrs()).unwrap();
    let a = tid(0x0100_0001);
    t.acquire_or_enqueue(1, a);
    t.acquire_or_enqueue(1, a);
    assert_eq!(t.recursion_discards_count(), 0);
    assert_eq!(t.release_and_wake_next(1, a), MutexRelease::Freed);
    assert_eq!(
        t.recursion_discards_count(),
        1,
        "the depth owed back on a cond-wake re-acquire must not vanish unwitnessed",
    );
}

#[test]
fn a_release_of_a_singly_held_mutex_is_not_a_recursion_discard() {
    let mut t = MutexTable::new();
    t.create_with_id(1, recursive_attrs()).unwrap();
    let a = tid(0x0100_0001);
    t.acquire_or_enqueue(1, a);
    t.acquire_or_enqueue(1, a);
    assert!(t.unlock_decrement(1, a), "drain the one recursive hold");
    assert_eq!(t.release_and_wake_next(1, a), MutexRelease::Freed);
    assert_eq!(t.recursion_discards_count(), 0);
    // A refused release must not count either.
    assert_eq!(
        t.release_and_wake_next(99, a),
        MutexRelease::Unknown,
        "unknown id",
    );
    assert_eq!(t.recursion_discards_count(), 0);
}

#[test]
fn a_non_owner_release_leaves_the_recursion_witness_and_the_depth_alone() {
    let mut t = MutexTable::new();
    t.create_with_id(1, recursive_attrs()).unwrap();
    let (owner, other) = (tid(0x0100_0001), tid(0x0100_0002));
    t.acquire_or_enqueue(1, owner);
    t.acquire_or_enqueue(1, owner);
    assert_eq!(t.release_and_wake_next(1, other), MutexRelease::NotOwner);
    assert_eq!(t.recursion_discards_count(), 0);
    assert_eq!(t.lookup(1).unwrap().lock_count(), 1);
    assert_eq!(t.lookup(1).unwrap().owner(), Some(owner));
}

#[test]
fn purge_waiters_of_reports_pairs_in_id_then_enqueue_order() {
    let mut t = MutexTable::new();
    let (a, b, c) = (tid(0x0100_0001), tid(0x0100_0002), tid(0x0100_0003));
    for id in [2, 1] {
        t.create_with_id(id, default_attrs()).unwrap();
        t.try_acquire(id, c);
        t.enqueue_waiter(id, b).unwrap();
        t.enqueue_waiter(id, a).unwrap();
    }
    let dead: std::collections::BTreeSet<_> = [a, b].into_iter().collect();
    assert_eq!(
        t.purge_waiters_of(&dead),
        vec![(1, b), (1, a), (2, b), (2, a)],
        "ids ascend; within one mutex the queue order stands",
    );
    assert!(t.lookup(1).unwrap().waiters().is_empty());
    assert!(t.lookup(2).unwrap().waiters().is_empty());
}

#[test]
fn purge_waiters_of_leaves_ownership_and_lock_count_alone() {
    let mut t = MutexTable::new();
    t.create_with_id(1, recursive_attrs()).unwrap();
    let owner = tid(0x0100_0001);
    let dead_waiter = tid(0x0100_0002);
    t.acquire_or_enqueue(1, owner);
    t.acquire_or_enqueue(1, owner);
    t.enqueue_waiter(1, dead_waiter).unwrap();
    let dead: std::collections::BTreeSet<_> = [dead_waiter].into_iter().collect();
    assert_eq!(t.purge_waiters_of(&dead), vec![(1, dead_waiter)]);
    let e = t.lookup(1).unwrap();
    assert_eq!(e.owner(), Some(owner));
    assert_eq!(e.lock_count(), 1);
}

#[test]
fn a_purged_waiter_never_inherits_the_mutex_on_release() {
    let mut t = MutexTable::new();
    t.create_with_id(1, default_attrs()).unwrap();
    let (owner, dead, alive) = (tid(0x0100_0001), tid(0x0100_0002), tid(0x0100_0003));
    t.try_acquire(1, owner);
    t.enqueue_waiter(1, dead).unwrap();
    t.enqueue_waiter(1, alive).unwrap();
    let purged: std::collections::BTreeSet<_> = [dead].into_iter().collect();
    assert_eq!(t.purge_waiters_of(&purged), vec![(1, dead)]);
    assert_eq!(
        t.release_and_wake_next(1, owner),
        MutexRelease::Transferred { new_owner: alive },
    );
}

#[test]
fn purge_waiters_of_an_empty_set_is_a_no_op_on_the_state_hash() {
    let mut t = MutexTable::new();
    t.create_with_id(1, default_attrs()).unwrap();
    t.try_acquire(1, tid(0x0100_0001));
    t.enqueue_waiter(1, tid(0x0100_0002)).unwrap();
    let before = t.state_hash();
    assert!(t
        .purge_waiters_of(&std::collections::BTreeSet::new())
        .is_empty());
    assert_eq!(t.state_hash(), before);
}

#[test]
fn ids_owned_by_lists_only_the_mutexes_a_named_thread_holds() {
    let mut t = MutexTable::new();
    let (dead, alive) = (tid(0x0100_0001), tid(0x0100_0002));
    for id in [3, 1, 2, 4] {
        t.create_with_id(id, default_attrs()).unwrap();
    }
    t.try_acquire(3, dead);
    t.try_acquire(1, alive);
    t.try_acquire(2, dead);
    // Mutex 4 stays unowned.
    let set: std::collections::BTreeSet<_> = [dead].into_iter().collect();
    assert_eq!(t.ids_owned_by(&set), vec![2, 3], "ascending id order");
}

#[test]
fn ids_owned_by_an_empty_set_finds_nothing() {
    let mut t = MutexTable::new();
    t.create_with_id(1, default_attrs()).unwrap();
    t.try_acquire(1, tid(0x0100_0001));
    assert!(t
        .ids_owned_by(&std::collections::BTreeSet::new())
        .is_empty());
}

#[test]
fn a_waiter_only_thread_is_not_reported_as_an_owner() {
    let mut t = MutexTable::new();
    t.create_with_id(1, default_attrs()).unwrap();
    let (owner, waiter) = (tid(0x0100_0001), tid(0x0100_0002));
    t.try_acquire(1, owner);
    t.enqueue_waiter(1, waiter).unwrap();
    let set: std::collections::BTreeSet<_> = [waiter].into_iter().collect();
    assert!(t.ids_owned_by(&set).is_empty());
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
fn remove_waiter_unqueues_only_the_named_thread() {
    let mut t = MutexTable::new();
    t.create_with_id(1, default_attrs()).unwrap();
    let owner = tid(0x0100_0001);
    let w1 = tid(0x0100_0002);
    let w2 = tid(0x0100_0003);
    let w3 = tid(0x0100_0004);
    t.try_acquire(1, owner);
    t.enqueue_waiter(1, w1).unwrap();
    t.enqueue_waiter(1, w2).unwrap();
    t.enqueue_waiter(1, w3).unwrap();
    assert!(t.remove_waiter(1, w2));
    let remaining: Vec<_> = t.lookup(1).unwrap().waiters().iter().collect();
    assert_eq!(remaining, vec![w1, w3], "FIFO order of the rest survives");
    assert_eq!(
        t.lookup(1).unwrap().owner(),
        Some(owner),
        "removal must not grant or move ownership"
    );
    assert_eq!(
        t.release_and_wake_next(1, owner),
        MutexRelease::Transferred { new_owner: w1 },
        "post-cancel unlock must transfer to the surviving FIFO head"
    );
}

#[test]
fn remove_waiter_unknown_id_or_unparked_thread_is_false() {
    let mut t = MutexTable::new();
    assert!(!t.remove_waiter(99, tid(0x0100_0001)));
    t.create_with_id(1, default_attrs()).unwrap();
    assert!(!t.remove_waiter(1, tid(0x0100_0001)));
}

#[test]
fn remove_waiter_does_not_remove_the_owner() {
    let mut t = MutexTable::new();
    t.create_with_id(1, default_attrs()).unwrap();
    let owner = tid(0x0100_0001);
    t.try_acquire(1, owner);
    assert!(
        !t.remove_waiter(1, owner),
        "the owner is not on the waiter list"
    );
    assert_eq!(t.lookup(1).unwrap().owner(), Some(owner));
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
fn state_hash_distinguishes_an_outstanding_recursion_count() {
    let mut a = MutexTable::new();
    let mut b = MutexTable::new();
    a.create_with_id(1, recursive_attrs()).unwrap();
    b.create_with_id(1, recursive_attrs()).unwrap();
    let owner = tid(0x0100_0001);
    a.acquire_or_enqueue(1, owner);
    b.acquire_or_enqueue(1, owner);
    b.acquire_or_enqueue(1, owner);
    assert_ne!(a.state_hash(), b.state_hash());
}

#[test]
fn state_hash_returns_to_baseline_after_relock_then_decrement() {
    // Gate edge: a count back at zero folds no bytes, so the hash
    // reads as if the re-lock never happened.
    let mut t = MutexTable::new();
    t.create_with_id(1, recursive_attrs()).unwrap();
    let owner = tid(0x0100_0001);
    t.acquire_or_enqueue(1, owner);
    let baseline = t.state_hash();
    t.acquire_or_enqueue(1, owner);
    assert_ne!(baseline, t.state_hash());
    assert!(t.unlock_decrement(1, owner));
    assert_eq!(baseline, t.state_hash());
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
