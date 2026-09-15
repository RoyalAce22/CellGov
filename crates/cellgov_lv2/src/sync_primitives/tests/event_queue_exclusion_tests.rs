//! `mutual_exclusion_break` names a queue that holds buffered payloads
//! and parked waiters at once.

use super::*;

fn payload() -> EventPayload {
    EventPayload {
        source: 1,
        data1: 2,
        data2: 3,
        data3: 4,
    }
}

#[test]
fn a_queue_holding_one_side_or_none_reports_no_break() {
    let mut t = EventQueueTable::new();
    assert!(t.create_with_id(1, 4));
    assert_eq!(t.mutual_exclusion_break(1), None);
    t.send_and_wake_or_enqueue(1, payload());
    assert_eq!(t.mutual_exclusion_break(1), None);
    assert_eq!(t.mutual_exclusion_break(2), None);

    assert!(t.create_with_id(3, 4));
    t.enqueue_waiter(3, PpuThreadId::new(0x100), 0x2000)
        .expect("enqueue");
    assert_eq!(t.mutual_exclusion_break(3), None);
}

/// The setup trips a debug assertion in `enqueue_waiter`, so this test
/// compiles in a release build only.
#[cfg(not(debug_assertions))]
#[test]
fn a_queue_holding_both_sides_reports_both_counts() {
    let mut t = EventQueueTable::new();
    assert!(t.create_with_id(1, 4));
    t.send_and_wake_or_enqueue(1, payload());
    t.send_and_wake_or_enqueue(1, payload());
    t.enqueue_waiter(1, PpuThreadId::new(0x100), 0x2000)
        .expect("enqueue");
    assert_eq!(t.mutual_exclusion_break(1), Some((2, 1)));
}
