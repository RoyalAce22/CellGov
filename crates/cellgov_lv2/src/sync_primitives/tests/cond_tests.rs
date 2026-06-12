//! Condition-variable table tests -- mutex binding, FIFO waiter order, and create/destroy error paths.

use super::*;

fn tid(raw: u64) -> PpuThreadId {
    PpuThreadId::new(raw)
}

#[test]
fn fresh_table_is_empty() {
    let t = CondTable::new();
    assert!(t.is_empty());
    assert_eq!(t.len(), 0);
    assert!(t.lookup(1).is_none());
}

#[test]
fn create_with_id_stores_binding() {
    let mut t = CondTable::new();
    assert_eq!(
        t.create_with_id(0x4000_0010, 7, CondMutexKind::LwMutex),
        Ok(())
    );
    let e = t.lookup(0x4000_0010).unwrap();
    assert_eq!(e.mutex_id(), 7);
    assert_eq!(e.mutex_kind(), CondMutexKind::LwMutex);
    assert!(e.waiters().is_empty());
}

#[cfg(debug_assertions)]
#[test]
#[should_panic(expected = "redundantly registered")]
fn create_with_id_redundant_registration_fires_debug_assert() {
    let mut t = CondTable::new();
    t.create_with_id(5, 1, CondMutexKind::LwMutex).unwrap();
    let _ = t.create_with_id(5, 1, CondMutexKind::LwMutex);
}

#[cfg(debug_assertions)]
#[test]
#[should_panic(expected = "collides")]
fn create_with_id_id_collision_fires_debug_assert() {
    let mut t = CondTable::new();
    t.create_with_id(5, 1, CondMutexKind::LwMutex).unwrap();
    let _ = t.create_with_id(5, 2, CondMutexKind::Mutex);
}

#[cfg(not(debug_assertions))]
#[test]
fn create_with_id_redundant_registration_returns_err_in_release() {
    let mut t = CondTable::new();
    t.create_with_id(5, 1, CondMutexKind::LwMutex).unwrap();
    assert_eq!(
        t.create_with_id(5, 1, CondMutexKind::LwMutex),
        Err(CondCreateError::RedundantRegistration)
    );
    let e = t.lookup(5).unwrap();
    assert_eq!(e.mutex_id(), 1);
    assert_eq!(e.mutex_kind(), CondMutexKind::LwMutex);
}

#[cfg(not(debug_assertions))]
#[test]
fn create_with_id_id_collision_returns_err_in_release() {
    let mut t = CondTable::new();
    t.create_with_id(5, 1, CondMutexKind::LwMutex).unwrap();
    assert_eq!(
        t.create_with_id(5, 2, CondMutexKind::Mutex),
        Err(CondCreateError::IdCollision {
            collision: crate::sync_primitives::IdCollision { id: 5 },
            existing_mutex_id: 1,
            existing_mutex_kind: CondMutexKind::LwMutex,
        })
    );
    let e = t.lookup(5).unwrap();
    assert_eq!(e.mutex_id(), 1);
    assert_eq!(e.mutex_kind(), CondMutexKind::LwMutex);
}

#[cfg(not(debug_assertions))]
#[test]
fn enqueue_waiter_duplicate_returns_err_in_release() {
    let mut t = CondTable::new();
    t.create_with_id(5, 1, CondMutexKind::LwMutex).unwrap();
    t.enqueue_waiter(5, tid(0x0100_0001)).unwrap();
    assert_eq!(
        t.enqueue_waiter(5, tid(0x0100_0001)),
        Err(CondEnqueueError::DuplicateWaiter)
    );
    assert_eq!(t.lookup(5).unwrap().waiters().len(), 1);
}

#[test]
fn destroy_removes_entry() {
    let mut t = CondTable::new();
    t.create_with_id(5, 1, CondMutexKind::LwMutex).unwrap();
    assert!(t.destroy(5).is_some());
    assert!(t.lookup(5).is_none());
    assert!(t.destroy(5).is_none());
}

#[cfg(debug_assertions)]
#[test]
#[should_panic(expected = "destroyed with")]
fn destroy_with_parked_waiters_fires_debug_assert() {
    let mut t = CondTable::new();
    t.create_with_id(5, 1, CondMutexKind::LwMutex).unwrap();
    t.enqueue_waiter(5, tid(0x0100_0001)).unwrap();
    let _ = t.destroy(5);
}

#[test]
fn enqueue_waiter_fifo_order() {
    let mut t = CondTable::new();
    t.create_with_id(5, 1, CondMutexKind::LwMutex).unwrap();
    assert_eq!(t.enqueue_waiter(5, tid(0x0100_0001)), Ok(()));
    assert_eq!(t.enqueue_waiter(5, tid(0x0100_0002)), Ok(()));
    assert_eq!(t.enqueue_waiter(5, tid(0x0100_0003)), Ok(()));
    let seen: Vec<_> = t.lookup(5).unwrap().waiters().iter().collect();
    assert_eq!(
        seen,
        vec![tid(0x0100_0001), tid(0x0100_0002), tid(0x0100_0003)],
    );
}

#[test]
fn enqueue_waiter_unknown_id_returns_unknown_error() {
    let mut t = CondTable::new();
    assert_eq!(
        t.enqueue_waiter(99, tid(0x0100_0001)),
        Err(CondEnqueueError::UnknownId)
    );
}

#[cfg(debug_assertions)]
#[test]
#[should_panic(expected = "duplicate enqueue")]
fn enqueue_waiter_duplicate_fires_debug_assert() {
    let mut t = CondTable::new();
    t.create_with_id(5, 1, CondMutexKind::LwMutex).unwrap();
    t.enqueue_waiter(5, tid(0x0100_0001)).unwrap();
    let _ = t.enqueue_waiter(5, tid(0x0100_0001));
}

#[test]
fn signal_one_pops_head() {
    let mut t = CondTable::new();
    t.create_with_id(5, 1, CondMutexKind::LwMutex).unwrap();
    t.enqueue_waiter(5, tid(0x0100_0001)).unwrap();
    t.enqueue_waiter(5, tid(0x0100_0002)).unwrap();
    t.enqueue_waiter(5, tid(0x0100_0003)).unwrap();
    assert_eq!(t.signal_one(5), Some(tid(0x0100_0001)));
    assert_eq!(t.signal_one(5), Some(tid(0x0100_0002)));
    assert_eq!(t.signal_one(5), Some(tid(0x0100_0003)));
    assert_eq!(t.signal_one(5), None);
}

#[test]
fn signal_one_with_no_waiters_is_lost() {
    let mut t = CondTable::new();
    t.create_with_id(5, 1, CondMutexKind::LwMutex).unwrap();
    assert_eq!(t.signal_one(5), None);
    assert_eq!(t.signal_one(5), None);
    t.enqueue_waiter(5, tid(0x0100_0001)).unwrap();
    assert_eq!(t.lookup(5).unwrap().waiters().len(), 1);
}

#[test]
fn signal_one_unknown_id_returns_none() {
    let mut t = CondTable::new();
    assert_eq!(t.signal_one(99), None);
}

#[test]
fn signal_all_drains_in_fifo_order() {
    let mut t = CondTable::new();
    t.create_with_id(5, 1, CondMutexKind::LwMutex).unwrap();
    t.enqueue_waiter(5, tid(0x0100_0001)).unwrap();
    t.enqueue_waiter(5, tid(0x0100_0002)).unwrap();
    t.enqueue_waiter(5, tid(0x0100_0003)).unwrap();
    let woken = t.signal_all(5).unwrap();
    assert_eq!(
        woken,
        vec![tid(0x0100_0001), tid(0x0100_0002), tid(0x0100_0003)],
    );
    assert!(t.lookup(5).unwrap().waiters().is_empty());
}

#[test]
fn signal_all_with_no_waiters_returns_some_empty() {
    let mut t = CondTable::new();
    t.create_with_id(5, 1, CondMutexKind::LwMutex).unwrap();
    assert_eq!(t.signal_all(5), Some(Vec::new()));
}

#[test]
fn signal_all_unknown_id_returns_none() {
    let mut t = CondTable::new();
    assert_eq!(t.signal_all(99), None);
}

#[test]
fn signal_to_removes_specific_waiter() {
    let mut t = CondTable::new();
    t.create_with_id(5, 1, CondMutexKind::LwMutex).unwrap();
    t.enqueue_waiter(5, tid(0x0100_0001)).unwrap();
    t.enqueue_waiter(5, tid(0x0100_0002)).unwrap();
    t.enqueue_waiter(5, tid(0x0100_0003)).unwrap();
    assert_eq!(t.signal_to(5, tid(0x0100_0002)), Ok(()));
    let remaining: Vec<_> = t.lookup(5).unwrap().waiters().iter().collect();
    assert_eq!(remaining, vec![tid(0x0100_0001), tid(0x0100_0003)]);
}

#[test]
fn signal_to_missing_target_returns_target_not_waiting() {
    let mut t = CondTable::new();
    t.create_with_id(5, 1, CondMutexKind::LwMutex).unwrap();
    t.enqueue_waiter(5, tid(0x0100_0001)).unwrap();
    assert_eq!(
        t.signal_to(5, tid(0x0100_0099)),
        Err(CondSignalToError::TargetNotWaiting)
    );
}

#[test]
fn signal_to_unknown_id_returns_unknown() {
    let mut t = CondTable::new();
    assert_eq!(
        t.signal_to(99, tid(0x0100_0001)),
        Err(CondSignalToError::UnknownId)
    );
}

#[test]
fn heavy_mutex_binding_is_preserved() {
    let mut t = CondTable::new();
    t.create_with_id(0x4000_0020, 0x4000_0005, CondMutexKind::Mutex)
        .unwrap();
    let e = t.lookup(0x4000_0020).unwrap();
    assert_eq!(e.mutex_kind(), CondMutexKind::Mutex);
    assert_eq!(e.mutex_id(), 0x4000_0005);
}

#[test]
fn state_hash_empty_is_stable() {
    let a = CondTable::new();
    let b = CondTable::new();
    assert_eq!(a.state_hash(), b.state_hash());
}

#[test]
fn state_hash_distinguishes_mutex_kind() {
    let mut a = CondTable::new();
    let mut b = CondTable::new();
    a.create_with_id(5, 1, CondMutexKind::LwMutex).unwrap();
    b.create_with_id(5, 1, CondMutexKind::Mutex).unwrap();
    assert_ne!(a.state_hash(), b.state_hash());
}

#[test]
fn state_hash_distinguishes_mutex_id() {
    let mut a = CondTable::new();
    let mut b = CondTable::new();
    a.create_with_id(5, 1, CondMutexKind::LwMutex).unwrap();
    b.create_with_id(5, 2, CondMutexKind::LwMutex).unwrap();
    assert_ne!(a.state_hash(), b.state_hash());
}

#[test]
fn state_hash_distinguishes_waiter_order() {
    let mut a = CondTable::new();
    let mut b = CondTable::new();
    a.create_with_id(5, 1, CondMutexKind::LwMutex).unwrap();
    b.create_with_id(5, 1, CondMutexKind::LwMutex).unwrap();
    a.enqueue_waiter(5, tid(0x0100_0001)).unwrap();
    a.enqueue_waiter(5, tid(0x0100_0002)).unwrap();
    b.enqueue_waiter(5, tid(0x0100_0002)).unwrap();
    b.enqueue_waiter(5, tid(0x0100_0001)).unwrap();
    assert_ne!(a.state_hash(), b.state_hash());
}

#[test]
fn state_hash_ignores_ephemeral_signal_attempts() {
    let mut a = CondTable::new();
    let mut b = CondTable::new();
    a.create_with_id(5, 1, CondMutexKind::LwMutex).unwrap();
    b.create_with_id(5, 1, CondMutexKind::LwMutex).unwrap();
    let _ = a.signal_one(5);
    let _ = a.signal_all(5);
    let _ = a.signal_to(5, tid(0x0100_0099));
    assert_eq!(a.state_hash(), b.state_hash());
}
