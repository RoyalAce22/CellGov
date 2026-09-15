//! A receive from a queue that holds payloads and parked waiters at
//! once logs a named invariant break before it drains the queue.
//!
//! The setup trips a debug assertion in the queue table, so these
//! tests compile in a release build only.

#![cfg(not(debug_assertions))]

use super::*;
use crate::ppu_thread::PpuThreadId;

const SITE: &str = "sync_primitives.event_queue.mutual_exclusion";
const QUEUE: u32 = 0x4000_0001;

fn broken_host() -> Lv2Host {
    let mut host = Lv2Host::new();
    let queues = &mut host.state.event_queues;
    assert!(queues.create_with_id(QUEUE, 8));
    queues.send_and_wake_or_enqueue(
        QUEUE,
        EventPayload {
            source: 1,
            data1: 2,
            data2: 3,
            data3: 4,
        },
    );
    queues
        .enqueue_waiter(QUEUE, PpuThreadId::new(0x100), 0x2000)
        .expect("enqueue");
    host
}

#[test]
fn a_tryreceive_names_the_break() {
    let mut host = broken_host();
    host.dispatch_event_queue_tryreceive(
        QUEUE,
        0x3000,
        4,
        0x3100,
        UnitId::new(0),
        GuestTicks::ZERO,
    );
    assert_eq!(host.invariant_break_site_count(SITE), 1);
    let first = host
        .observability()
        .first_invariant_break
        .as_deref()
        .unwrap_or_default();
    assert!(first.starts_with(SITE), "{first}");
}

#[test]
fn a_receive_names_the_break() {
    let mut host = broken_host();
    crate::host::test_support::seed_primary_ppu(&mut host, UnitId::new(0));
    host.dispatch_event_queue_receive(QUEUE, 0x3000, UnitId::new(0), GuestTicks::ZERO);
    assert_eq!(host.invariant_break_site_count(SITE), 1);
}
