//! Event-flag table tests -- AND/OR wait modes, clear semantics, and FIFO wake order.

use super::*;

fn tid(raw: u64) -> PpuThreadId {
    PpuThreadId::new(raw)
}

#[test]
fn fresh_table_is_empty() {
    let t = EventFlagTable::new();
    assert!(t.is_empty());
}

#[test]
fn try_wait_and_mode_requires_all_bits() {
    let mut t = EventFlagTable::new();
    t.create_with_id(1, 0b1010).unwrap();
    assert_eq!(
        t.try_wait(1, 0b1000, EventFlagWaitMode::AndNoClear),
        Some(EventFlagWait::Matched { observed: 0b1010 }),
    );
    assert_eq!(t.lookup(1).unwrap().bits(), 0b1010);
    assert_eq!(
        t.try_wait(1, 0b0101, EventFlagWaitMode::AndNoClear),
        Some(EventFlagWait::NoMatch),
    );
}

#[test]
fn try_wait_or_mode_requires_any_bit() {
    let mut t = EventFlagTable::new();
    t.create_with_id(1, 0b0010).unwrap();
    assert_eq!(
        t.try_wait(1, 0b1010, EventFlagWaitMode::OrNoClear),
        Some(EventFlagWait::Matched { observed: 0b0010 }),
    );
    assert_eq!(
        t.try_wait(1, 0b1100, EventFlagWaitMode::OrNoClear),
        Some(EventFlagWait::NoMatch),
    );
}

#[test]
fn try_wait_clear_mode_clears_matched_bits() {
    let mut t = EventFlagTable::new();
    t.create_with_id(1, 0b1111).unwrap();
    assert_eq!(
        t.try_wait(1, 0b1010, EventFlagWaitMode::AndClear),
        Some(EventFlagWait::Matched { observed: 0b1111 }),
    );
    assert_eq!(t.lookup(1).unwrap().bits(), 0b0101);
}

#[test]
fn set_with_no_waiters_just_ors_bits() {
    let mut t = EventFlagTable::new();
    t.create_with_id(1, 0b0001).unwrap();
    let woken = t.set_and_wake(1, 0b1010).unwrap();
    assert!(woken.is_empty());
    assert_eq!(t.lookup(1).unwrap().bits(), 0b1011);
}

#[test]
fn set_wakes_matching_waiter_in_fifo_order() {
    let mut t = EventFlagTable::new();
    t.create_with_id(1, 0).unwrap();
    t.enqueue_waiter(
        1,
        tid(0x0100_0001),
        0b0001,
        EventFlagWaitMode::AndNoClear,
        0x2000,
    )
    .unwrap();
    t.enqueue_waiter(
        1,
        tid(0x0100_0002),
        0b0010,
        EventFlagWaitMode::AndNoClear,
        0x2020,
    )
    .unwrap();
    t.enqueue_waiter(
        1,
        tid(0x0100_0003),
        0b1000,
        EventFlagWaitMode::AndNoClear,
        0x2040,
    )
    .unwrap();
    let woken = t.set_and_wake(1, 0b0011).unwrap();
    assert_eq!(
        woken,
        vec![
            EventFlagWake {
                thread: tid(0x0100_0001),
                observed: 0b0011,
                result_ptr: 0x2000,
            },
            EventFlagWake {
                thread: tid(0x0100_0002),
                observed: 0b0011,
                result_ptr: 0x2020,
            },
        ],
    );
    assert_eq!(t.lookup(1).unwrap().waiters().len(), 1);
    assert_eq!(t.lookup(1).unwrap().waiters()[0].thread, tid(0x0100_0003));
}

#[test]
fn set_with_clear_waiter_clears_bits_before_next_waiter_check() {
    let mut t = EventFlagTable::new();
    t.create_with_id(1, 0).unwrap();
    t.enqueue_waiter(
        1,
        tid(0x0100_0001),
        0b0001,
        EventFlagWaitMode::AndClear,
        0x2000,
    )
    .unwrap();
    t.enqueue_waiter(
        1,
        tid(0x0100_0002),
        0b0001,
        EventFlagWaitMode::AndNoClear,
        0x2020,
    )
    .unwrap();
    let woken = t.set_and_wake(1, 0b0001).unwrap();
    assert_eq!(
        woken,
        vec![EventFlagWake {
            thread: tid(0x0100_0001),
            observed: 0b0001,
            result_ptr: 0x2000,
        }],
    );
    assert_eq!(t.lookup(1).unwrap().bits(), 0);
    assert_eq!(t.lookup(1).unwrap().waiters().len(), 1);
}

#[test]
fn set_and_wake_clear_then_noclear_shows_different_observed() {
    let mut t = EventFlagTable::new();
    t.create_with_id(1, 0).unwrap();
    t.enqueue_waiter(
        1,
        tid(0x0100_0001),
        0b0001,
        EventFlagWaitMode::AndClear,
        0x2000,
    )
    .unwrap();
    t.enqueue_waiter(
        1,
        tid(0x0100_0002),
        0b0010,
        EventFlagWaitMode::AndNoClear,
        0x2020,
    )
    .unwrap();
    let woken = t.set_and_wake(1, 0b0011).unwrap();
    assert_eq!(
        woken,
        vec![
            EventFlagWake {
                thread: tid(0x0100_0001),
                observed: 0b0011,
                result_ptr: 0x2000,
            },
            EventFlagWake {
                thread: tid(0x0100_0002),
                observed: 0b0010,
                result_ptr: 0x2020,
            },
        ],
    );
    assert_eq!(t.lookup(1).unwrap().bits(), 0b0010);
}

#[test]
fn clear_bits_masks_without_waking() {
    let mut t = EventFlagTable::new();
    t.create_with_id(1, 0b0111).unwrap();
    t.enqueue_waiter(
        1,
        tid(0x0100_0001),
        0b1000,
        EventFlagWaitMode::AndNoClear,
        0x2000,
    )
    .unwrap();
    assert!(t.clear_bits(1, 0b0101));
    assert_eq!(t.lookup(1).unwrap().bits(), 0b0101);
    assert_eq!(t.lookup(1).unwrap().waiters().len(), 1);
}

#[test]
fn unknown_id_returns_none() {
    let mut t = EventFlagTable::new();
    assert!(t.try_wait(99, 0b1, EventFlagWaitMode::AndNoClear).is_none());
    assert!(t.set_and_wake(99, 0b1).is_none());
    assert!(!t.clear_bits(99, 0b1));
    assert_eq!(
        t.enqueue_waiter(99, tid(1), 0b1, EventFlagWaitMode::AndNoClear, 0x100),
        Err(EventFlagEnqueueError::UnknownId),
    );
}

#[test]
fn set_wakes_each_waiter_with_its_own_result_ptr() {
    let mut t = EventFlagTable::new();
    t.create_with_id(1, 0).unwrap();
    t.enqueue_waiter(
        1,
        tid(0x0100_0001),
        0b0001,
        EventFlagWaitMode::AndNoClear,
        0x1000,
    )
    .unwrap();
    t.enqueue_waiter(
        1,
        tid(0x0100_0002),
        0b0010,
        EventFlagWaitMode::AndNoClear,
        0x2000,
    )
    .unwrap();
    t.enqueue_waiter(
        1,
        tid(0x0100_0003),
        0b0100,
        EventFlagWaitMode::AndNoClear,
        0x3000,
    )
    .unwrap();
    let woken = t.set_and_wake(1, 0b0111).unwrap();
    assert_eq!(
        woken,
        vec![
            EventFlagWake {
                thread: tid(0x0100_0001),
                observed: 0b0111,
                result_ptr: 0x1000,
            },
            EventFlagWake {
                thread: tid(0x0100_0002),
                observed: 0b0111,
                result_ptr: 0x2000,
            },
            EventFlagWake {
                thread: tid(0x0100_0003),
                observed: 0b0111,
                result_ptr: 0x3000,
            },
        ],
    );
}

#[test]
fn state_hash_distinguishes_waiter_mode() {
    let mut a = EventFlagTable::new();
    let mut b = EventFlagTable::new();
    a.create_with_id(1, 0).unwrap();
    b.create_with_id(1, 0).unwrap();
    a.enqueue_waiter(
        1,
        tid(0x0100_0001),
        0b1,
        EventFlagWaitMode::AndClear,
        0x2000,
    )
    .unwrap();
    b.enqueue_waiter(
        1,
        tid(0x0100_0001),
        0b1,
        EventFlagWaitMode::AndNoClear,
        0x2000,
    )
    .unwrap();
    assert_ne!(a.state_hash(), b.state_hash());
}

#[test]
#[cfg(debug_assertions)]
#[should_panic(expected = "already present")]
fn create_with_id_panics_on_collision_in_debug() {
    let mut t = EventFlagTable::new();
    t.create_with_id(1, 0xAA).unwrap();
    let _ = t.create_with_id(1, 0xBB);
}

#[test]
#[cfg(debug_assertions)]
#[should_panic(expected = "duplicate enqueue")]
fn duplicate_enqueue_panics_in_debug() {
    let mut t = EventFlagTable::new();
    t.create_with_id(1, 0).unwrap();
    t.enqueue_waiter(
        1,
        tid(0x0100_0001),
        0b1,
        EventFlagWaitMode::AndNoClear,
        0x2000,
    )
    .unwrap();
    let _ = t.enqueue_waiter(
        1,
        tid(0x0100_0001),
        0b1,
        EventFlagWaitMode::AndNoClear,
        0x2000,
    );
}

#[test]
#[cfg(debug_assertions)]
#[should_panic(expected = "already match")]
fn enqueue_on_already_matching_bits_panics_in_debug() {
    let mut t = EventFlagTable::new();
    t.create_with_id(1, 0b1111).unwrap();
    let _ = t.enqueue_waiter(
        1,
        tid(0x0100_0001),
        0b0001,
        EventFlagWaitMode::AndNoClear,
        0x2000,
    );
}

#[test]
#[cfg(debug_assertions)]
#[should_panic(expected = "destroyed with")]
fn destroy_with_parked_waiters_panics_in_debug() {
    let mut t = EventFlagTable::new();
    t.create_with_id(1, 0).unwrap();
    t.enqueue_waiter(
        1,
        tid(0x0100_0001),
        0b1,
        EventFlagWaitMode::AndNoClear,
        0x2000,
    )
    .unwrap();
    let _ = t.destroy(1);
}

#[test]
#[cfg(not(debug_assertions))]
fn create_with_id_returns_collision_err_in_release() {
    let mut t = EventFlagTable::new();
    t.create_with_id(1, 0xAA).unwrap();
    assert_eq!(
        t.create_with_id(1, 0xBB),
        Err(EventFlagCreateError::IdCollision(
            crate::sync_primitives::IdCollision { id: 1 }
        )),
    );
    assert_eq!(t.lookup(1).unwrap().init(), 0xAA);
}

#[test]
#[cfg(not(debug_assertions))]
fn duplicate_enqueue_returns_err_in_release() {
    let mut t = EventFlagTable::new();
    t.create_with_id(1, 0).unwrap();
    t.enqueue_waiter(
        1,
        tid(0x0100_0001),
        0b1,
        EventFlagWaitMode::AndNoClear,
        0x2000,
    )
    .unwrap();
    assert_eq!(
        t.enqueue_waiter(
            1,
            tid(0x0100_0001),
            0b1,
            EventFlagWaitMode::AndNoClear,
            0x2000,
        ),
        Err(EventFlagEnqueueError::DuplicateWaiter),
    );
    assert_eq!(t.lookup(1).unwrap().waiters().len(), 1);
}

#[test]
#[cfg(not(debug_assertions))]
fn enqueue_on_already_matching_bits_still_parks_in_release() {
    let mut t = EventFlagTable::new();
    t.create_with_id(1, 0b1111).unwrap();
    t.enqueue_waiter(
        1,
        tid(0x0100_0001),
        0b0001,
        EventFlagWaitMode::AndNoClear,
        0x2000,
    )
    .unwrap();
    assert_eq!(t.lookup(1).unwrap().waiters().len(), 1);
    let woken = t.set_and_wake(1, 0).unwrap();
    assert_eq!(woken.len(), 1);
}
