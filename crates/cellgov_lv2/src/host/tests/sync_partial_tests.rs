//! Every sync-primitive table reaches the host's sync-state partial.

use super::*;
use crate::dispatch::CondMutexKind;

#[test]
fn every_primitive_table_moves_the_host_partial() {
    let mut host = Lv2Host::new();
    let mut before = host.sync_partial();
    let mut check = |host: &Lv2Host, what: &str| {
        let now = host.sync_partial();
        assert_eq!(now, host.sync_partial_from_scratch(), "after {what}");
        assert_ne!(now, before, "{what} did not reach the host partial");
        before = now;
    };
    host.state.lwmutexes.create().unwrap();
    check(&host, "lwmutex");
    host.state
        .mutexes
        .create_with_id(0x10, Default::default())
        .unwrap();
    check(&host, "mutex");
    host.state.semaphores.create_with_id(0x20, 0, 1).unwrap();
    check(&host, "semaphore");
    host.state
        .conds
        .create_with_id(0x30, 0x10, CondMutexKind::Mutex)
        .unwrap();
    check(&host, "cond");
    host.state.event_queues.create_with_id(0x40, 4);
    check(&host, "event queue");
    host.state.event_ports.create_with_id(0x50, 1, 0);
    check(&host, "event port");
    host.state.event_flags.create_with_id(0x60, 0).unwrap();
    check(&host, "event flag");
}

#[test]
fn event_queue_waiter_order_moves_the_host_partial() {
    use crate::ppu_thread::PpuThreadId;
    let build = |order: [u64; 2]| {
        let mut host = Lv2Host::new();
        assert!(host.state.event_queues.create_with_id(0x40, 4));
        for thread in order {
            host.state
                .event_queues
                .enqueue_waiter(0x40, PpuThreadId::new(thread), 0x2000)
                .unwrap();
        }
        host.sync_partial()
    };
    assert_ne!(
        build([0x0100_0001, 0x0100_0002]),
        build([0x0100_0002, 0x0100_0001])
    );
}

#[test]
fn event_flag_waiter_order_moves_the_host_partial() {
    use crate::ppu_thread::{EventFlagWaitMode, PpuThreadId};
    let build = |order: [u64; 2]| {
        let mut host = Lv2Host::new();
        host.state.event_flags.create_with_id(0x60, 0).unwrap();
        for thread in order {
            host.state
                .event_flags
                .enqueue_waiter(
                    0x60,
                    PpuThreadId::new(thread),
                    0b1,
                    EventFlagWaitMode::AndClear,
                    0x2000,
                )
                .unwrap();
        }
        host.sync_partial()
    };
    assert_ne!(
        build([0x0100_0001, 0x0100_0002]),
        build([0x0100_0002, 0x0100_0001])
    );
}
