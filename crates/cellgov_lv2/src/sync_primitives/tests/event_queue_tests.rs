//! Event-queue table tests -- payload FIFO ordering and direct hand-off to parked waiters.

use super::*;

fn tid(raw: u64) -> PpuThreadId {
    PpuThreadId::new(raw)
}

fn pl(source: u64) -> EventPayload {
    EventPayload {
        source,
        data1: source + 1,
        data2: source + 2,
        data3: source + 3,
    }
}

#[test]
fn fresh_table_is_empty() {
    let t = EventQueueTable::new();
    assert!(t.is_empty());
}

#[test]
fn create_rejects_zero_size() {
    let mut t = EventQueueTable::new();
    assert!(!t.create_with_id(1, 0));
    assert!(t.lookup(1).is_none());
}

#[test]
fn create_rejects_collision() {
    let mut t = EventQueueTable::new();
    assert!(t.create_with_id(1, 4));
    assert!(!t.create_with_id(1, 4));
}

#[test]
fn try_receive_empty_queue_returns_empty() {
    let mut t = EventQueueTable::new();
    t.create_with_id(1, 4);
    assert_eq!(t.try_receive(1), Some(EventQueueReceive::Empty));
}

#[test]
fn try_receive_unknown_is_none() {
    let mut t = EventQueueTable::new();
    assert!(t.try_receive(99).is_none());
}

#[test]
fn send_with_no_waiters_enqueues_and_try_receive_returns_it() {
    let mut t = EventQueueTable::new();
    t.create_with_id(1, 4);
    assert_eq!(
        t.send_and_wake_or_enqueue(1, pl(10)),
        EventQueueSend::Enqueued
    );
    assert_eq!(t.try_receive(1), Some(EventQueueReceive::Delivered(pl(10))),);
    assert_eq!(t.try_receive(1), Some(EventQueueReceive::Empty));
}

#[test]
fn send_preserves_arrival_order_across_multiple_payloads() {
    let mut t = EventQueueTable::new();
    t.create_with_id(1, 4);
    assert_eq!(
        t.send_and_wake_or_enqueue(1, pl(1)),
        EventQueueSend::Enqueued
    );
    assert_eq!(
        t.send_and_wake_or_enqueue(1, pl(2)),
        EventQueueSend::Enqueued
    );
    assert_eq!(
        t.send_and_wake_or_enqueue(1, pl(3)),
        EventQueueSend::Enqueued
    );
    assert_eq!(t.try_receive(1), Some(EventQueueReceive::Delivered(pl(1))),);
    assert_eq!(t.try_receive(1), Some(EventQueueReceive::Delivered(pl(2))),);
    assert_eq!(t.try_receive(1), Some(EventQueueReceive::Delivered(pl(3))),);
}

#[test]
fn send_with_parked_waiter_hands_off_directly_and_does_not_enqueue() {
    let mut t = EventQueueTable::new();
    t.create_with_id(1, 4);
    t.enqueue_waiter(1, tid(0x0100_0001), 0x2000).unwrap();
    assert_eq!(
        t.send_and_wake_or_enqueue(1, pl(42)),
        EventQueueSend::Woke {
            new_owner: tid(0x0100_0001),
            out_ptr: 0x2000,
            payload: pl(42),
        },
    );
    assert!(t.lookup(1).unwrap().is_empty());
    assert!(t.lookup(1).unwrap().waiters().is_empty());
}

#[test]
fn send_wakes_waiters_in_fifo_order() {
    let mut t = EventQueueTable::new();
    t.create_with_id(1, 4);
    t.enqueue_waiter(1, tid(0x0100_0001), 0x2000).unwrap();
    t.enqueue_waiter(1, tid(0x0100_0002), 0x2020).unwrap();
    t.enqueue_waiter(1, tid(0x0100_0003), 0x2040).unwrap();
    assert_eq!(
        t.send_and_wake_or_enqueue(1, pl(10)),
        EventQueueSend::Woke {
            new_owner: tid(0x0100_0001),
            out_ptr: 0x2000,
            payload: pl(10),
        },
    );
    assert_eq!(
        t.send_and_wake_or_enqueue(1, pl(20)),
        EventQueueSend::Woke {
            new_owner: tid(0x0100_0002),
            out_ptr: 0x2020,
            payload: pl(20),
        },
    );
    assert_eq!(
        t.send_and_wake_or_enqueue(1, pl(30)),
        EventQueueSend::Woke {
            new_owner: tid(0x0100_0003),
            out_ptr: 0x2040,
            payload: pl(30),
        },
    );
    assert_eq!(
        t.send_and_wake_or_enqueue(1, pl(40)),
        EventQueueSend::Enqueued
    );
}

#[test]
fn send_preserves_per_waiter_out_ptr_even_when_zero() {
    let mut t = EventQueueTable::new();
    t.create_with_id(1, 4);
    t.enqueue_waiter(1, tid(0x0100_0001), 0).unwrap();
    assert_eq!(
        t.send_and_wake_or_enqueue(1, pl(42)),
        EventQueueSend::Woke {
            new_owner: tid(0x0100_0001),
            out_ptr: 0,
            payload: pl(42),
        },
    );
}

#[test]
fn send_when_queue_full_returns_full() {
    let mut t = EventQueueTable::new();
    t.create_with_id(1, 2);
    t.send_and_wake_or_enqueue(1, pl(1));
    t.send_and_wake_or_enqueue(1, pl(2));
    assert_eq!(t.send_and_wake_or_enqueue(1, pl(3)), EventQueueSend::Full);
    assert_eq!(t.lookup(1).unwrap().len(), 2);
}

#[test]
fn send_unknown_id_is_unknown() {
    let mut t = EventQueueTable::new();
    assert_eq!(
        t.send_and_wake_or_enqueue(99, pl(1)),
        EventQueueSend::Unknown
    );
}

#[test]
fn try_receive_batch_drains_up_to_max_in_fifo_order() {
    let mut t = EventQueueTable::new();
    t.create_with_id(1, 10);
    t.send_and_wake_or_enqueue(1, pl(1));
    t.send_and_wake_or_enqueue(1, pl(2));
    t.send_and_wake_or_enqueue(1, pl(3));
    let batch = t.try_receive_batch(1, 2).unwrap();
    assert_eq!(batch, vec![pl(1), pl(2)]);
    assert_eq!(t.lookup(1).unwrap().len(), 1);
    let batch2 = t.try_receive_batch(1, 4).unwrap();
    assert_eq!(batch2, vec![pl(3)]);
    assert_eq!(t.try_receive_batch(1, 4).unwrap(), vec![]);
}

#[test]
fn try_receive_batch_unknown_is_none() {
    let mut t = EventQueueTable::new();
    assert!(t.try_receive_batch(99, 4).is_none());
}

#[test]
fn enqueue_waiter_unknown_id_returns_err() {
    let mut t = EventQueueTable::new();
    assert_eq!(
        t.enqueue_waiter(99, tid(0x0100_0001), 0x2000),
        Err(EventQueueEnqueueError::UnknownId),
    );
}

#[test]
fn is_inert_reflects_both_axes() {
    let mut t = EventQueueTable::new();
    t.create_with_id(1, 4);
    assert!(t.lookup(1).unwrap().is_inert());
    t.send_and_wake_or_enqueue(1, pl(1));
    assert!(!t.lookup(1).unwrap().is_empty());
    assert!(!t.lookup(1).unwrap().is_inert());
    let _ = t.try_receive(1);
    assert!(t.lookup(1).unwrap().is_inert());
    t.enqueue_waiter(1, tid(0x0100_0001), 0x2000).unwrap();
    assert!(t.lookup(1).unwrap().is_empty());
    assert!(!t.lookup(1).unwrap().is_inert());
}

#[test]
fn state_hash_distinguishes_payload_order() {
    let mut a = EventQueueTable::new();
    let mut b = EventQueueTable::new();
    a.create_with_id(1, 4);
    b.create_with_id(1, 4);
    a.send_and_wake_or_enqueue(1, pl(1));
    a.send_and_wake_or_enqueue(1, pl(2));
    b.send_and_wake_or_enqueue(1, pl(2));
    b.send_and_wake_or_enqueue(1, pl(1));
    assert_ne!(a.state_hash(), b.state_hash());
}

#[test]
#[cfg(debug_assertions)]
#[should_panic(expected = "duplicate enqueue")]
fn duplicate_enqueue_panics_in_debug() {
    let mut t = EventQueueTable::new();
    t.create_with_id(1, 4);
    t.enqueue_waiter(1, tid(0x0100_0001), 0x2000).unwrap();
    let _ = t.enqueue_waiter(1, tid(0x0100_0001), 0x2000);
}

#[test]
#[cfg(debug_assertions)]
#[should_panic(expected = "payload(s)")]
fn enqueue_when_payloads_present_panics_in_debug() {
    let mut t = EventQueueTable::new();
    t.create_with_id(1, 4);
    t.send_and_wake_or_enqueue(1, pl(1));
    let _ = t.enqueue_waiter(1, tid(0x0100_0001), 0x2000);
}

#[test]
#[cfg(debug_assertions)]
#[should_panic(expected = "destroyed with")]
fn destroy_with_parked_waiters_panics_in_debug() {
    let mut t = EventQueueTable::new();
    t.create_with_id(1, 4);
    t.enqueue_waiter(1, tid(0x0100_0001), 0x2000).unwrap();
    let _ = t.destroy(1);
}

#[test]
#[cfg(not(debug_assertions))]
fn duplicate_enqueue_returns_err_in_release() {
    let mut t = EventQueueTable::new();
    t.create_with_id(1, 4);
    t.enqueue_waiter(1, tid(0x0100_0001), 0x2000).unwrap();
    assert_eq!(
        t.enqueue_waiter(1, tid(0x0100_0001), 0x2000),
        Err(EventQueueEnqueueError::DuplicateWaiter),
    );
    assert_eq!(t.lookup(1).unwrap().waiters().len(), 1);
}
