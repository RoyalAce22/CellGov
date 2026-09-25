//! The runtime-owned sync sources keep their partials of the sync-state
//! sum through every path that changes them.

use cellgov_event::UnitId;
use cellgov_lv2::{Lv2BlockReason, PendingResponse};
use cellgov_mem::GuestMemory;
use cellgov_time::{Budget, GuestTicks};

use crate::runtime::spaces::AddressSpaceId;
use crate::runtime::state::Runtime;
use crate::timer_queue::TimerWakeKind;

const S1: AddressSpaceId = AddressSpaceId::new(1);

/// `change` changes exactly one source.
fn step(rt: &mut Runtime, before: &mut u64, what: &str, change: impl FnOnce(&mut Runtime)) {
    change(rt);
    let now = rt.sync_state_hash();
    assert_eq!(now, rt.sync_state_hash_from_scratch(), "after {what}");
    assert_ne!(now, *before, "{what} did not move the hash");
    *before = now;
}

#[test]
fn every_runtime_source_moves_the_hash_and_keeps_its_partial() {
    let mut rt = Runtime::new(GuestMemory::new(16), Budget::new(4), 100);
    let empty = rt.sync_state_hash();
    let mut h = empty;
    let unit = UnitId::new(3);
    step(&mut rt, &mut h, "response insert", |rt| {
        assert!(rt
            .syscall_responses
            .insert(unit, PendingResponse::ReturnCode { code: 5 })
            .is_none());
    });
    step(&mut rt, &mut h, "response take", |rt| {
        rt.syscall_responses.try_take(unit);
    });
    assert_eq!(h, empty, "insert-then-take returns the sum");
    step(&mut rt, &mut h, "timer insert", |rt| {
        rt.timer_wakes.insert(
            GuestTicks::new(10),
            unit,
            TimerWakeKind::SyncWait(Lv2BlockReason::Semaphore { id: 2 }),
        );
    });
    step(&mut rt, &mut h, "second timer insert", |rt| {
        rt.timer_wakes
            .insert(GuestTicks::new(20), UnitId::new(4), TimerWakeKind::Sleep);
    });
    step(&mut rt, &mut h, "timer cancel", |rt| {
        rt.timer_wakes.cancel(unit);
    });
    step(&mut rt, &mut h, "timer pop_due", |rt| {
        assert_eq!(rt.timer_wakes.pop_due(GuestTicks::new(20)).len(), 1);
    });
    assert_eq!(h, empty, "a drained queue returns the sum");
    step(&mut rt, &mut h, "cursor put", |rt| {
        rt.rsx_cursor.set_put(0x40)
    });
    step(&mut rt, &mut h, "cursor get", |rt| {
        rt.rsx_cursor.set_get(0x20)
    });
    step(&mut rt, &mut h, "cursor reference", |rt| {
        rt.rsx_cursor.set_reference(7)
    });
    step(&mut rt, &mut h, "flip request", |rt| {
        rt.rsx_flip.request_flip(1)
    });
    step(&mut rt, &mut h, "flip handler", |rt| {
        rt.rsx_flip.set_handler(0x100)
    });
    step(&mut rt, &mut h, "semaphore offset", |rt| {
        rt.rsx_sem_offset = 0x30
    });
    step(&mut rt, &mut h, "space create", |rt| {
        rt.create_address_space(S1).unwrap()
    });
    step(&mut rt, &mut h, "unit tag", |rt| {
        rt.assign_unit_space(unit, S1).unwrap()
    });
    step(&mut rt, &mut h, "shared mapping", |rt| {
        rt.register_shared_mapping(
            9,
            0x1_0000,
            &[(AddressSpaceId::BOOT, 0x1_0000), (S1, 0x1_0000)],
        )
        .unwrap()
    });
    step(&mut rt, &mut h, "lv2 mutex create", |rt| {
        rt.lv2_host_mut()
            .mutexes_mut()
            .create_with_id(0x10, Default::default())
            .unwrap()
    });
    step(&mut rt, &mut h, "unit untag", |rt| {
        rt.assign_unit_space(unit, AddressSpaceId::BOOT).unwrap()
    });
}

#[test]
fn two_shared_mappings_that_exchange_views_hash_differently() {
    let build = |views: [(AddressSpaceId, u64); 2]| {
        let mut rt = Runtime::new(GuestMemory::new(16), Budget::new(4), 100);
        rt.create_address_space(S1).unwrap();
        rt.register_shared_mapping(9, 0x1_0000, &views).unwrap();
        assert_eq!(rt.sync_state_hash(), rt.sync_state_hash_from_scratch());
        rt.sync_state_hash()
    };
    let a = build([(AddressSpaceId::BOOT, 0x1_0000), (S1, 0x2_0000)]);
    let b = build([(S1, 0x2_0000), (AddressSpaceId::BOOT, 0x1_0000)]);
    let c = build([(AddressSpaceId::BOOT, 0x2_0000), (S1, 0x1_0000)]);
    assert_ne!(a, b, "view order is replication order");
    assert_ne!(a, c, "a view's base belongs to its space");
}
