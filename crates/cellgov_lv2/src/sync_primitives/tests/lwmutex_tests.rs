//! Lightweight-mutex table tests -- signal-based acquire/release, id allocation, and waiter parking.

use super::*;

fn tid(raw: u64) -> PpuThreadId {
    PpuThreadId::new(raw)
}

#[test]
fn fresh_table_is_empty() {
    let t = LwMutexTable::new();
    assert!(t.is_empty());
    assert_eq!(t.len(), 0);
    assert!(t.lookup(1).is_none());
}

#[test]
fn id_allocator_is_monotonic_and_starts_at_one() {
    let mut a = LwMutexIdAllocator::new();
    assert_eq!(a.allocate(), Some(1));
    assert_eq!(a.allocate(), Some(2));
    assert_eq!(a.allocate(), Some(3));
}

#[test]
fn id_allocator_exhaustion() {
    let mut a = LwMutexIdAllocator { next: u32::MAX };
    assert_eq!(a.allocate(), None);
    assert_eq!(a.allocate(), None);
}

#[test]
fn id_allocator_last_handed_out_is_u32_max_minus_one() {
    let mut a = LwMutexIdAllocator { next: u32::MAX - 1 };
    assert_eq!(a.allocate(), Some(u32::MAX - 1));
    assert_eq!(a.allocate(), None);
}

#[test]
fn create_returns_monotonic_ids() {
    let mut t = LwMutexTable::new();
    let id1 = t.create().unwrap();
    let id2 = t.create().unwrap();
    let id3 = t.create().unwrap();
    assert!(id1 < id2 && id2 < id3);
    assert_eq!(t.len(), 3);
}

#[test]
fn destroy_removes_entry_and_does_not_recycle_id() {
    let mut t = LwMutexTable::new();
    let id1 = t.create().unwrap();
    assert!(t.destroy(id1).is_some());
    assert!(t.lookup(id1).is_none());
    let id2 = t.create().unwrap();
    assert!(id2 > id1);
}

#[test]
fn fresh_entry_is_unsignaled() {
    // A freshly created entry starts un-signaled. Locks reaching
    // the kernel always park; only an unlock against an empty
    // queue sets the signal so the next contender can pass.
    let mut t = LwMutexTable::new();
    let id = t.create().unwrap();
    assert!(!t.lookup(id).unwrap().signaled());
}

#[test]
fn try_acquire_unsignaled_is_contended() {
    let mut t = LwMutexTable::new();
    let id = t.create().unwrap();
    let caller = tid(0x0100_0001);
    assert_eq!(t.try_acquire(id, caller), Some(LwMutexAcquire::Contended));
    assert!(!t.lookup(id).unwrap().signaled());
}

#[test]
fn try_acquire_consumes_signal_set_by_unlock() {
    // After an unlock-with-no-waiters sets the signal, the next
    // try_acquire consumes it.
    let mut t = LwMutexTable::new();
    let id = t.create().unwrap();
    let a = tid(0x0100_0001);
    t.release_and_wake_next(id, a);
    assert!(t.lookup(id).unwrap().signaled());
    assert_eq!(t.try_acquire(id, a), Some(LwMutexAcquire::Acquired));
    assert!(!t.lookup(id).unwrap().signaled());
}

#[test]
fn try_acquire_unknown_id_is_none() {
    let mut t = LwMutexTable::new();
    assert!(t.try_acquire(99, tid(0x0100_0001)).is_none());
}

#[test]
fn acquire_or_enqueue_unsignaled_parks() {
    let mut t = LwMutexTable::new();
    let id = t.create().unwrap();
    let a = tid(0x0100_0001);
    assert_eq!(
        t.acquire_or_enqueue(id, a),
        LwMutexAcquireOrEnqueue::Enqueued,
    );
    assert_eq!(t.lookup(id).unwrap().waiters().len(), 1);
}

#[test]
fn acquire_or_enqueue_consumes_signal_when_pending() {
    let mut t = LwMutexTable::new();
    let id = t.create().unwrap();
    // Set the signal via an unlock against an empty queue.
    t.release_and_wake_next(id, tid(0x0100_0001));
    assert_eq!(
        t.acquire_or_enqueue(id, tid(0x0100_0002)),
        LwMutexAcquireOrEnqueue::Acquired,
    );
    assert!(!t.lookup(id).unwrap().signaled());
}

#[test]
fn acquire_or_enqueue_already_parked_is_would_deadlock() {
    let mut t = LwMutexTable::new();
    let id = t.create().unwrap();
    let waiter = tid(0x0100_0002);
    t.acquire_or_enqueue(id, waiter);
    assert_eq!(
        t.acquire_or_enqueue(id, waiter),
        LwMutexAcquireOrEnqueue::WouldDeadlock,
    );
    assert_eq!(t.lookup(id).unwrap().waiters().len(), 1);
}

#[test]
fn acquire_or_enqueue_unknown_id_is_unknown() {
    let mut t = LwMutexTable::new();
    assert_eq!(
        t.acquire_or_enqueue(99, tid(0x0100_0001)),
        LwMutexAcquireOrEnqueue::Unknown,
    );
}

#[test]
fn enqueue_waiter_preserves_fifo_order() {
    let mut t = LwMutexTable::new();
    let id = t.create().unwrap();
    // Consume the initial signal first.
    t.try_acquire(id, tid(0x0100_0001));
    t.enqueue_waiter(id, tid(0x0100_0002)).unwrap();
    t.enqueue_waiter(id, tid(0x0100_0003)).unwrap();
    t.enqueue_waiter(id, tid(0x0100_0004)).unwrap();
    let seen: Vec<_> = t.lookup(id).unwrap().waiters().iter().collect();
    assert_eq!(
        seen,
        vec![tid(0x0100_0002), tid(0x0100_0003), tid(0x0100_0004)],
    );
}

#[test]
fn enqueue_waiter_unknown_id_returns_err() {
    let mut t = LwMutexTable::new();
    assert_eq!(
        t.enqueue_waiter(99, tid(0x0100_0001)),
        Err(LwMutexEnqueueError::UnknownId),
    );
}

#[test]
fn release_without_waiters_signals() {
    let mut t = LwMutexTable::new();
    let id = t.create().unwrap();
    let a = tid(0x0100_0001);
    t.try_acquire(id, a);
    assert_eq!(t.release_and_wake_next(id, a), LwMutexRelease::Signaled);
    assert!(t.lookup(id).unwrap().signaled());
}

#[test]
fn release_with_waiters_transfers_to_head() {
    let mut t = LwMutexTable::new();
    let id = t.create().unwrap();
    let owner = tid(0x0100_0001);
    let w1 = tid(0x0100_0002);
    let w2 = tid(0x0100_0003);
    t.try_acquire(id, owner);
    t.enqueue_waiter(id, w1).unwrap();
    t.enqueue_waiter(id, w2).unwrap();
    assert_eq!(
        t.release_and_wake_next(id, owner),
        LwMutexRelease::Transferred { new_owner: w1 },
    );
    // Transfer does not signal; the wake hands off directly.
    assert!(!t.lookup(id).unwrap().signaled());
    let remaining: Vec<_> = t.lookup(id).unwrap().waiters().iter().collect();
    assert_eq!(remaining, vec![w2]);
}

#[test]
fn release_unknown_id_is_unknown() {
    let mut t = LwMutexTable::new();
    assert_eq!(
        t.release_and_wake_next(99, tid(0x0100_0001)),
        LwMutexRelease::Unknown,
    );
}

#[test]
fn unlock_then_acquire_via_signal() {
    // Unlock against an empty queue sets the signal; the next
    // acquire consumes it without parking.
    let mut t = LwMutexTable::new();
    let id = t.create().unwrap();
    let a = tid(0x0100_0001);
    assert_eq!(t.release_and_wake_next(id, a), LwMutexRelease::Signaled);
    let b = tid(0x0100_0002);
    assert_eq!(
        t.acquire_or_enqueue(id, b),
        LwMutexAcquireOrEnqueue::Acquired,
    );
    assert!(!t.lookup(id).unwrap().signaled());
}

#[test]
fn state_hash_empty_is_stable() {
    let a = LwMutexTable::new();
    let b = LwMutexTable::new();
    assert_eq!(a.state_hash(), b.state_hash());
}

#[test]
fn state_hash_distinguishes_signaled_state() {
    let mut a = LwMutexTable::new();
    let mut b = LwMutexTable::new();
    let id_a = a.create().unwrap();
    b.create().unwrap();
    assert_eq!(a.state_hash(), b.state_hash());
    // Unlock-with-no-waiters sets the signal on `a` only.
    a.release_and_wake_next(id_a, tid(0x0100_0001));
    assert_ne!(a.state_hash(), b.state_hash());
}

#[test]
fn state_hash_distinguishes_waiter_order() {
    let mut a = LwMutexTable::new();
    let mut b = LwMutexTable::new();
    let id_a = a.create().unwrap();
    let id_b = b.create().unwrap();
    a.try_acquire(id_a, tid(0x0100_0001));
    b.try_acquire(id_b, tid(0x0100_0001));
    a.enqueue_waiter(id_a, tid(0x0100_0002)).unwrap();
    a.enqueue_waiter(id_a, tid(0x0100_0003)).unwrap();
    b.enqueue_waiter(id_b, tid(0x0100_0003)).unwrap();
    b.enqueue_waiter(id_b, tid(0x0100_0002)).unwrap();
    assert_ne!(a.state_hash(), b.state_hash());
}

#[test]
fn state_hash_distinguishes_allocator_cursor() {
    let mut a = LwMutexTable::new();
    let mut b = LwMutexTable::new();
    a.create().unwrap();
    let a_temp = a.create().unwrap();
    a.destroy(a_temp);
    b.create().unwrap();
    assert_eq!(a.len(), b.len());
    assert_ne!(a.state_hash(), b.state_hash());
}

#[test]
#[cfg(debug_assertions)]
#[should_panic(expected = "duplicate enqueue")]
fn duplicate_enqueue_panics_in_debug() {
    let mut t = LwMutexTable::new();
    let id = t.create().unwrap();
    let waiter = tid(0x0100_0002);
    t.try_acquire(id, tid(0x0100_0001));
    t.enqueue_waiter(id, waiter).unwrap();
    let _ = t.enqueue_waiter(id, waiter);
}

#[test]
#[cfg(debug_assertions)]
#[should_panic(expected = "destroyed with")]
fn destroy_with_parked_waiters_panics_in_debug() {
    let mut t = LwMutexTable::new();
    let id = t.create().unwrap();
    t.try_acquire(id, tid(0x0100_0001));
    t.enqueue_waiter(id, tid(0x0100_0002)).unwrap();
    let _ = t.destroy(id);
}

#[test]
#[cfg(not(debug_assertions))]
fn duplicate_enqueue_returns_err_in_release() {
    let mut t = LwMutexTable::new();
    let id = t.create().unwrap();
    let waiter = tid(0x0100_0002);
    t.try_acquire(id, tid(0x0100_0001));
    t.enqueue_waiter(id, waiter).unwrap();
    assert_eq!(
        t.enqueue_waiter(id, waiter),
        Err(LwMutexEnqueueError::DuplicateWaiter),
    );
    assert_eq!(t.lookup(id).unwrap().waiters().len(), 1);
}

#[test]
#[cfg(not(debug_assertions))]
fn destroy_with_parked_waiters_returns_entry_unchanged_in_release() {
    let mut t = LwMutexTable::new();
    let id = t.create().unwrap();
    let waiter1 = tid(0x0100_0002);
    let waiter2 = tid(0x0100_0003);
    t.try_acquire(id, tid(0x0100_0001));
    t.enqueue_waiter(id, waiter1).unwrap();
    t.enqueue_waiter(id, waiter2).unwrap();
    let removed = t.destroy(id).unwrap();
    let parked: Vec<_> = removed.waiters().iter().collect();
    assert_eq!(parked, vec![waiter1, waiter2]);
    assert!(t.lookup(id).is_none());
}
