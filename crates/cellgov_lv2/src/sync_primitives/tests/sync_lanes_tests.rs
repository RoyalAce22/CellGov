//! The sync-primitive tables keep their partials of the sync-state sum
//! through every path that changes them.

use std::collections::BTreeSet;

use super::*;
use crate::dispatch::CondMutexKind;
use crate::ppu_thread::{EventFlagWaitMode, PpuThreadId};

const A: PpuThreadId = PpuThreadId::new(0x0100_0001);
const B: PpuThreadId = PpuThreadId::new(0x0100_0002);

/// Apply `$change` to `$t`; the partial must move and equal the rebuild,
/// in release too.
macro_rules! step {
    ($t:ident, $h:ident, $what:expr, $change:expr) => {{
        let _ = $change;
        let now = $t.sync_partial();
        assert_eq!(now, $t.sync_partial_from_scratch(), "after {}", $what);
        assert_ne!(now, $h, "{} did not move the partial", $what);
        $h = now;
        let _ = $h;
    }};
}

#[test]
fn lwmutex_changes_move_the_partial_and_the_allocator_outlives_destroy() {
    let mut t = LwMutexTable::new();
    let empty = t.sync_partial();
    let mut h = empty;
    step!(t, h, "create", t.create().unwrap());
    step!(t, h, "release signals", t.release_and_wake_next(1, A));
    step!(
        t,
        h,
        "acquire consumes the signal",
        t.acquire_or_enqueue(1, A)
    );
    step!(t, h, "contended acquire parks", t.acquire_or_enqueue(1, A));
    step!(t, h, "enqueue", t.enqueue_waiter(1, B));
    step!(t, h, "remove shifts the survivor", t.remove_waiter(1, A));
    step!(t, h, "purge", t.purge_waiters_of(&BTreeSet::from([B])));
    step!(t, h, "destroy", t.destroy(1));
    assert_ne!(h, empty, "the id allocator keeps its cursor");
}

#[test]
fn mutex_changes_move_the_partial() {
    let mut t = MutexTable::new();
    let mut h = t.sync_partial();
    let attrs = MutexAttrs {
        recursive: true,
        ..Default::default()
    };
    step!(t, h, "create", t.create_with_id(0x10, attrs));
    step!(t, h, "acquire", t.acquire_or_enqueue(0x10, A));
    step!(t, h, "recursive relock", t.acquire_or_enqueue(0x10, A));
    step!(t, h, "enqueue", t.enqueue_waiter(0x10, B));
    step!(t, h, "unlock one hold", t.unlock_decrement(0x10, A));
    step!(t, h, "release transfers", t.release_and_wake_next(0x10, A));
    step!(t, h, "release frees", t.release_and_wake_next(0x10, B));
    step!(t, h, "destroy", t.destroy(0x10));
    assert_eq!(h, MutexTable::new().sync_partial());
}

#[test]
fn semaphore_changes_move_the_partial() {
    let mut t = SemaphoreTable::new();
    let mut h = t.sync_partial();
    step!(t, h, "create", t.create_with_id(0x20, 0, 5));
    step!(t, h, "post", t.post_and_wake_n(0x20, 1));
    step!(t, h, "wait", t.try_wait(0x20));
    step!(t, h, "enqueue", t.enqueue_waiter(0x20, A));
    step!(t, h, "second enqueue", t.enqueue_waiter(0x20, B));
    step!(t, h, "remove", t.remove_waiter(0x20, A));
    step!(t, h, "post wakes", t.post_and_wake_n(0x20, 1));
    step!(t, h, "destroy", t.destroy(0x20));
}

#[test]
fn cond_changes_move_the_partial() {
    let mut t = CondTable::new();
    let mut h = t.sync_partial();
    step!(
        t,
        h,
        "create",
        t.create_with_id(0x30, 0x10, CondMutexKind::Mutex)
    );
    step!(t, h, "enqueue", t.enqueue_waiter(0x30, A));
    step!(t, h, "second enqueue", t.enqueue_waiter(0x30, B));
    step!(t, h, "signal to", t.signal_to(0x30, A));
    step!(t, h, "signal one", t.signal_one(0x30));
    step!(t, h, "destroy", t.destroy(0x30));
}

#[test]
fn event_queue_changes_move_the_partial() {
    let mut t = EventQueueTable::new();
    let mut h = t.sync_partial();
    let payload = |n| EventPayload {
        source: n,
        data1: n + 1,
        data2: n + 2,
        data3: n + 3,
    };
    step!(t, h, "create", t.create_with_id(0x40, 4));
    step!(
        t,
        h,
        "send buffers",
        t.send_and_wake_or_enqueue(0x40, payload(1))
    );
    step!(
        t,
        h,
        "second send",
        t.send_and_wake_or_enqueue(0x40, payload(9))
    );
    step!(t, h, "receive", t.try_receive(0x40));
    step!(t, h, "drain", t.try_receive(0x40));
    step!(t, h, "enqueue", t.enqueue_waiter(0x40, A, 0x100));
    step!(t, h, "second enqueue", t.enqueue_waiter(0x40, B, 0x200));
    step!(t, h, "remove", t.remove_waiter(0x40, A));
    step!(
        t,
        h,
        "send wakes",
        t.send_and_wake_or_enqueue(0x40, payload(1))
    );
    step!(t, h, "destroy", t.destroy(0x40));
}

#[test]
fn event_port_changes_move_the_partial() {
    let mut t = EventPortTable::new();
    let mut h = t.sync_partial();
    step!(t, h, "create", t.create_with_id(0x50, 1, 7));
    step!(t, h, "connect", t.connect(0x50, 0x40, 1));
    step!(t, h, "disconnect", t.disconnect(0x50));
    step!(t, h, "connect again", t.connect(0x50, 0x40, 1));
    step!(t, h, "unbind", t.unbind_queue(0x40));
    step!(t, h, "destroy", t.destroy(0x50));
}

#[test]
fn event_flag_changes_move_the_partial() {
    let mut t = EventFlagTable::new();
    let mut h = t.sync_partial();
    step!(t, h, "create", t.create_with_id(0x60, 0));
    step!(
        t,
        h,
        "enqueue",
        t.enqueue_waiter(0x60, A, 1, EventFlagWaitMode::AndClear, 0x200)
    );
    step!(t, h, "set without waking", t.set_and_wake(0x60, 2));
    step!(t, h, "cancel", t.cancel_waiters(0x60));
    step!(t, h, "clear", t.clear_bits(0x60, 0));
    step!(t, h, "destroy", t.destroy(0x60));
}

#[test]
fn two_lwmutexes_that_exchange_states_hash_differently() {
    let build = |signaled: u32| {
        let mut t = LwMutexTable::new();
        t.create().unwrap();
        t.create().unwrap();
        let _ = t.release_and_wake_next(signaled, A);
        t.sync_partial()
    };
    assert_ne!(build(1), build(2));
}

#[test]
fn waiters_in_another_order_hash_differently() {
    let build = |first, second| {
        let mut t = SemaphoreTable::new();
        t.create_with_id(0x20, 0, 5).unwrap();
        t.enqueue_waiter(0x20, first).unwrap();
        t.enqueue_waiter(0x20, second).unwrap();
        t.sync_partial()
    };
    assert_ne!(build(A, B), build(B, A));
}

fn assert_all_distinct(partials: &[u128]) {
    for (i, a) in partials.iter().enumerate() {
        for b in &partials[i + 1..] {
            assert_ne!(a, b);
        }
    }
}

#[test]
fn every_cond_port_queue_and_flag_field_moves_the_partial() {
    let cond = |mutex_id, kind| {
        let mut t = CondTable::new();
        t.create_with_id(0x30, mutex_id, kind).unwrap();
        t.sync_partial()
    };
    assert_all_distinct(&[
        cond(0x10, CondMutexKind::Mutex),
        cond(0x11, CondMutexKind::Mutex),
        cond(0x10, CondMutexKind::LwMutex),
    ]);
    let port = |port_type, name| {
        let mut t = EventPortTable::new();
        t.create_with_id(0x50, port_type, name);
        t.sync_partial()
    };
    assert_all_distinct(&[port(1, 7), port(2, 7), port(1, 8)]);
    let queue = |size, out_ptr| {
        let mut t = EventQueueTable::new();
        t.create_with_id(0x40, size);
        t.enqueue_waiter(0x40, A, out_ptr).unwrap();
        t.sync_partial()
    };
    assert_all_distinct(&[queue(4, 0x100), queue(5, 0x100), queue(4, 0x108)]);
    let flag = |mask, mode, result_ptr| {
        let mut t = EventFlagTable::new();
        t.create_with_id(0x60, 0).unwrap();
        t.enqueue_waiter(0x60, A, mask, mode, result_ptr).unwrap();
        t.sync_partial()
    };
    assert_all_distinct(&[
        flag(1, EventFlagWaitMode::AndClear, 0x200),
        flag(2, EventFlagWaitMode::AndClear, 0x200),
        flag(1, EventFlagWaitMode::OrClear, 0x200),
        flag(1, EventFlagWaitMode::AndClear, 0x208),
    ]);
}

/// Computed outside the crate from the SplitMix64 key stream of the
/// sync-state lanes.
#[test]
fn lwmutex_partial_wire_format_golden() {
    let mut t = LwMutexTable::new();
    t.create().unwrap();
    t.create().unwrap();
    let _ = t.release_and_wake_next(1, A);
    t.enqueue_waiter(2, PpuThreadId::PRIMARY).unwrap();
    t.enqueue_waiter(2, A).unwrap();
    assert_eq!(t.sync_partial(), 0x62a2_e5a4_0f46_16d0_1acf_1b05_6475_5649);
}

/// Computed outside the crate from the SplitMix64 key stream of the
/// sync-state lanes.
#[test]
fn event_queue_partial_wire_format_golden() {
    let mut t = EventQueueTable::new();
    t.create_with_id(0x40, 4);
    let _ = t.send_and_wake_or_enqueue(
        0x40,
        EventPayload {
            source: 1,
            data1: 2,
            data2: 3,
            data3: 4,
        },
    );
    assert_eq!(t.sync_partial(), 0x4269_2c82_6c24_a617_d3cf_44ef_04f9_0000);
}
