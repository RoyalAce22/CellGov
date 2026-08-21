//! PpuThreadTable lifecycle tests -- create/finish/detach, join waiters, unit aliasing, and state-hash discrimination.

use super::*;
use crate::ppu_thread::{EventFlagWaitMode, GuestBlockReason};

fn dummy_attrs() -> PpuThreadAttrs {
    PpuThreadAttrs {
        entry: 0x10_0000,
        arg: 0,
        stack_base: 0xD000_0000,
        stack_size: 0x10000,
        priority: 1000,
        tls_base: 0x0020_0000,
    }
}

#[test]
fn new_table_is_empty() {
    let t = PpuThreadTable::new();
    assert!(t.is_empty());
    assert_eq!(t.len(), 0);
}

#[test]
fn insert_primary_records_unit_mapping() {
    let mut t = PpuThreadTable::new();
    t.insert_primary(UnitId::new(1), dummy_attrs());
    assert_eq!(t.len(), 1);
    let p = t.get(PpuThreadId::PRIMARY).unwrap();
    assert_eq!(p.id, PpuThreadId::PRIMARY);
    assert_eq!(p.unit_id, UnitId::new(1));
    assert_eq!(p.state, PpuThreadState::Runnable);
    assert_eq!(
        t.thread_id_for_unit(UnitId::new(1)),
        Some(PpuThreadId::PRIMARY),
    );
}

#[test]
#[should_panic(expected = "primary thread already inserted")]
fn double_primary_insert_panics() {
    let mut t = PpuThreadTable::new();
    t.insert_primary(UnitId::new(1), dummy_attrs());
    t.insert_primary(UnitId::new(2), dummy_attrs());
}

#[test]
#[should_panic(expected = "insert_primary called after create")]
fn insert_primary_after_create_panics() {
    let mut t = PpuThreadTable::new();
    t.create(UnitId::new(1), dummy_attrs()).unwrap();
    t.insert_primary(UnitId::new(2), dummy_attrs());
}

#[test]
#[cfg(debug_assertions)]
#[should_panic(expected = "already mapped to another thread")]
fn create_with_duplicate_unit_id_panics_in_debug() {
    let mut t = PpuThreadTable::new();
    t.create(UnitId::new(1), dummy_attrs()).unwrap();
    t.create(UnitId::new(1), dummy_attrs());
}

#[test]
fn create_allocates_above_primary() {
    let mut t = PpuThreadTable::new();
    t.insert_primary(UnitId::new(1), dummy_attrs());
    let c1 = t.create(UnitId::new(2), dummy_attrs()).unwrap();
    let c2 = t.create(UnitId::new(3), dummy_attrs()).unwrap();
    assert_eq!(c1.raw(), 0x0100_0001);
    assert_eq!(c2.raw(), 0x0100_0002);
    assert!(c1 > PpuThreadId::PRIMARY);
    assert!(c2 > c1);
}

#[test]
fn create_records_unit_and_attrs() {
    let mut t = PpuThreadTable::new();
    let mut attrs = dummy_attrs();
    attrs.arg = 0xdead_beef;
    let id = t.create(UnitId::new(5), attrs.clone()).unwrap();
    let thread = t.get(id).unwrap();
    assert_eq!(thread.unit_id, UnitId::new(5));
    assert_eq!(thread.attrs.arg, 0xdead_beef);
    assert_eq!(thread.state, PpuThreadState::Runnable);
    assert!(thread.join_waiters.is_empty());
    assert!(thread.exit_value.is_none());
    assert_eq!(t.get_by_unit(UnitId::new(5)).unwrap().id, id);
}

#[test]
fn get_by_unit_unknown_returns_none() {
    let t = PpuThreadTable::new();
    assert!(t.get_by_unit(UnitId::new(99)).is_none());
}

#[test]
fn mark_finished_sets_state_and_exit_value() {
    let mut t = PpuThreadTable::new();
    let id = t.create(UnitId::new(2), dummy_attrs()).unwrap();
    let waiters = t.mark_finished(id, 0x42);
    assert!(waiters.is_empty());
    let thread = t.get(id).unwrap();
    assert_eq!(thread.state, PpuThreadState::Finished);
    assert_eq!(thread.exit_value, Some(0x42));
}

#[test]
fn mark_finished_unknown_returns_empty() {
    let mut t = PpuThreadTable::new();
    let waiters = t.mark_finished(PpuThreadId::new(0x9999), 0);
    assert!(waiters.is_empty());
}

#[test]
#[cfg(debug_assertions)]
#[should_panic(expected = "already Finished")]
fn mark_finished_twice_panics_in_debug() {
    let mut t = PpuThreadTable::new();
    let id = t.create(UnitId::new(2), dummy_attrs()).unwrap();
    t.mark_finished(id, 0);
    t.mark_finished(id, 0);
}

#[test]
#[cfg(debug_assertions)]
#[should_panic(expected = "already Detached")]
fn mark_finished_after_detach_panics_in_debug() {
    let mut t = PpuThreadTable::new();
    let id = t.create(UnitId::new(2), dummy_attrs()).unwrap();
    t.detach(id);
    t.mark_finished(id, 0);
}

#[test]
fn add_join_waiter_and_mark_finished_drain_waiters() {
    let mut t = PpuThreadTable::new();
    t.insert_primary(UnitId::new(1), dummy_attrs());
    let child = t.create(UnitId::new(2), dummy_attrs()).unwrap();
    let third = t.create(UnitId::new(3), dummy_attrs()).unwrap();
    assert_eq!(
        t.add_join_waiter(child, PpuThreadId::PRIMARY),
        AddJoinWaiter::Parked,
    );
    assert_eq!(t.add_join_waiter(child, third), AddJoinWaiter::Parked);
    let waiters = t.mark_finished(child, 0);
    assert_eq!(waiters, vec![PpuThreadId::PRIMARY, third]);
    assert!(t.get(child).unwrap().join_waiters.is_empty());
}

#[test]
fn add_join_waiter_unknown_target_is_rejected() {
    let mut t = PpuThreadTable::new();
    assert_eq!(
        t.add_join_waiter(PpuThreadId::new(0x9999), PpuThreadId::PRIMARY),
        AddJoinWaiter::UnknownTarget,
    );
}

#[test]
fn add_join_waiter_self_join_is_rejected() {
    let mut t = PpuThreadTable::new();
    let id = t.create(UnitId::new(2), dummy_attrs()).unwrap();
    assert_eq!(t.add_join_waiter(id, id), AddJoinWaiter::SelfJoin);
    assert!(t.get(id).unwrap().join_waiters.is_empty());
}

#[test]
fn add_join_waiter_on_finished_target_is_rejected() {
    let mut t = PpuThreadTable::new();
    let target = t.create(UnitId::new(2), dummy_attrs()).unwrap();
    let waiter = t.create(UnitId::new(3), dummy_attrs()).unwrap();
    t.mark_finished(target, 0);
    assert_eq!(
        t.add_join_waiter(target, waiter),
        AddJoinWaiter::TargetAlreadyFinished,
    );
    assert!(t.get(target).unwrap().join_waiters.is_empty());
}

#[test]
fn add_join_waiter_on_detached_target_is_rejected() {
    let mut t = PpuThreadTable::new();
    let target = t.create(UnitId::new(2), dummy_attrs()).unwrap();
    let waiter = t.create(UnitId::new(3), dummy_attrs()).unwrap();
    t.detach(target);
    assert_eq!(
        t.add_join_waiter(target, waiter),
        AddJoinWaiter::TargetDetached,
    );
    assert!(t.get(target).unwrap().join_waiters.is_empty());
}

#[test]
fn take_join_waiters_without_state_change() {
    let mut t = PpuThreadTable::new();
    let child = t.create(UnitId::new(2), dummy_attrs()).unwrap();
    t.add_join_waiter(child, PpuThreadId::PRIMARY);
    let waiters = t.take_join_waiters(child);
    assert_eq!(waiters, vec![PpuThreadId::PRIMARY]);
    assert_eq!(t.get(child).unwrap().state, PpuThreadState::Runnable);
}

#[test]
fn purge_join_waiters_of_removes_only_the_named_waiters() {
    let mut t = PpuThreadTable::new();
    t.insert_primary(UnitId::new(1), dummy_attrs());
    let target = t.create(UnitId::new(2), dummy_attrs()).unwrap();
    let doomed = t.create(UnitId::new(3), dummy_attrs()).unwrap();
    let survivor = t.create(UnitId::new(4), dummy_attrs()).unwrap();
    t.add_join_waiter(target, doomed);
    t.add_join_waiter(target, survivor);

    let removed = t.purge_join_waiters_of(&std::collections::BTreeSet::from([doomed]));
    assert_eq!(removed, vec![(target, doomed)]);
    assert_eq!(t.get(target).unwrap().join_waiters, vec![survivor]);
}

#[test]
fn a_purged_joiner_is_not_returned_by_a_later_mark_finished() {
    let mut t = PpuThreadTable::new();
    t.insert_primary(UnitId::new(1), dummy_attrs());
    let target = t.create(UnitId::new(2), dummy_attrs()).unwrap();
    let doomed = t.create(UnitId::new(3), dummy_attrs()).unwrap();
    t.add_join_waiter(target, doomed);
    t.add_join_waiter(target, PpuThreadId::PRIMARY);

    t.purge_join_waiters_of(&std::collections::BTreeSet::from([doomed]));
    assert_eq!(t.mark_finished(target, 0x42), vec![PpuThreadId::PRIMARY]);
}

#[test]
fn purge_join_waiters_of_reports_pairs_in_ascending_target_order() {
    let mut t = PpuThreadTable::new();
    t.insert_primary(UnitId::new(1), dummy_attrs());
    let first = t.create(UnitId::new(2), dummy_attrs()).unwrap();
    let second = t.create(UnitId::new(3), dummy_attrs()).unwrap();
    let a = t.create(UnitId::new(4), dummy_attrs()).unwrap();
    let b = t.create(UnitId::new(5), dummy_attrs()).unwrap();
    // Park b before a on `second` so waiter order is not id order.
    t.add_join_waiter(second, b);
    t.add_join_waiter(second, a);
    t.add_join_waiter(first, a);

    let removed = t.purge_join_waiters_of(&std::collections::BTreeSet::from([a, b]));
    assert_eq!(
        removed,
        vec![(first, a), (second, b), (second, a)],
        "pairs must walk targets ascending and waiters in park order",
    );
    assert!(t.get(first).unwrap().join_waiters.is_empty());
    assert!(t.get(second).unwrap().join_waiters.is_empty());
}

#[test]
fn purging_a_thread_that_waits_on_nothing_reports_no_pairs() {
    let mut t = PpuThreadTable::new();
    t.insert_primary(UnitId::new(1), dummy_attrs());
    let target = t.create(UnitId::new(2), dummy_attrs()).unwrap();
    t.add_join_waiter(target, PpuThreadId::PRIMARY);
    let stranger = PpuThreadId::new(0x9999);

    assert!(t
        .purge_join_waiters_of(&std::collections::BTreeSet::from([stranger]))
        .is_empty());
    assert_eq!(
        t.get(target).unwrap().join_waiters,
        vec![PpuThreadId::PRIMARY],
    );
}

#[test]
fn purging_an_empty_set_leaves_every_join_list_intact() {
    let mut t = PpuThreadTable::new();
    t.insert_primary(UnitId::new(1), dummy_attrs());
    let target = t.create(UnitId::new(2), dummy_attrs()).unwrap();
    t.add_join_waiter(target, PpuThreadId::PRIMARY);
    let before = t.state_hash();

    assert!(t
        .purge_join_waiters_of(&std::collections::BTreeSet::new())
        .is_empty());
    assert_eq!(t.state_hash(), before);
}

#[test]
fn a_purged_waiter_that_is_itself_a_join_target_keeps_its_own_waiters() {
    let mut t = PpuThreadTable::new();
    t.insert_primary(UnitId::new(1), dummy_attrs());
    let doomed = t.create(UnitId::new(2), dummy_attrs()).unwrap();
    let other = t.create(UnitId::new(3), dummy_attrs()).unwrap();
    // `doomed` waits on `other`, and PRIMARY waits on `doomed`.
    t.add_join_waiter(other, doomed);
    t.add_join_waiter(doomed, PpuThreadId::PRIMARY);

    let removed = t.purge_join_waiters_of(&std::collections::BTreeSet::from([doomed]));
    assert_eq!(removed, vec![(other, doomed)]);
    // The purge is waiter-side only: PRIMARY stays parked on the
    // dead target and is still owed a wake by the caller.
    assert_eq!(
        t.get(doomed).unwrap().join_waiters,
        vec![PpuThreadId::PRIMARY],
    );
}

#[test]
fn detach_sets_state() {
    let mut t = PpuThreadTable::new();
    let id = t.create(UnitId::new(2), dummy_attrs()).unwrap();
    assert!(t.detach(id));
    assert_eq!(t.get(id).unwrap().state, PpuThreadState::Detached);
}

#[test]
fn detach_unknown_returns_false() {
    let mut t = PpuThreadTable::new();
    assert!(!t.detach(PpuThreadId::new(0x9999)));
}

#[test]
fn state_hash_distinguishes_every_guest_block_reason() {
    fn table_with_reason(reason: GuestBlockReason) -> u64 {
        let mut t = PpuThreadTable::new();
        let id = t.create(UnitId::new(1), dummy_attrs()).unwrap();
        t.get_mut(id).unwrap().state = PpuThreadState::Blocked(reason);
        t.state_hash()
    }
    let hashes = [
        table_with_reason(GuestBlockReason::WaitingOnJoin {
            target: PpuThreadId::PRIMARY,
        }),
        table_with_reason(GuestBlockReason::WaitingOnLwMutex { id: 1 }),
        table_with_reason(GuestBlockReason::WaitingOnMutex { id: 1 }),
        table_with_reason(GuestBlockReason::WaitingOnSemaphore { id: 1 }),
        table_with_reason(GuestBlockReason::WaitingOnEventQueue { id: 1 }),
        table_with_reason(GuestBlockReason::WaitingOnEventFlag {
            id: 1,
            mask: 0,
            mode: EventFlagWaitMode::AndNoClear,
        }),
        table_with_reason(GuestBlockReason::WaitingOnCond {
            cond_id: 1,
            mutex_id: 1,
        }),
    ];
    for (i, h_i) in hashes.iter().enumerate() {
        for (j, h_j) in hashes.iter().enumerate().skip(i + 1) {
            assert_ne!(h_i, h_j, "variants {i} and {j} hash-collided");
        }
    }
}

#[test]
fn state_hash_distinguishes_event_flag_wait_modes() {
    fn hash_with_mode(mode: EventFlagWaitMode) -> u64 {
        let mut t = PpuThreadTable::new();
        let id = t.create(UnitId::new(1), dummy_attrs()).unwrap();
        t.get_mut(id).unwrap().state =
            PpuThreadState::Blocked(GuestBlockReason::WaitingOnEventFlag {
                id: 1,
                mask: 0xAA,
                mode,
            });
        t.state_hash()
    }
    let a = hash_with_mode(EventFlagWaitMode::AndNoClear);
    let b = hash_with_mode(EventFlagWaitMode::AndClear);
    let c = hash_with_mode(EventFlagWaitMode::OrNoClear);
    let d = hash_with_mode(EventFlagWaitMode::OrClear);
    assert_ne!(a, b);
    assert_ne!(a, c);
    assert_ne!(a, d);
    assert_ne!(b, c);
    assert_ne!(b, d);
    assert_ne!(c, d);
}

#[test]
fn state_hash_empty_table_is_stable() {
    let a = PpuThreadTable::new();
    let b = PpuThreadTable::new();
    assert_eq!(a.state_hash(), b.state_hash());
}

#[test]
fn state_hash_differs_when_thread_added() {
    let empty = PpuThreadTable::new();
    let mut populated = PpuThreadTable::new();
    populated.create(UnitId::new(1), dummy_attrs()).unwrap();
    assert_ne!(empty.state_hash(), populated.state_hash());
}

#[test]
fn state_hash_changes_on_finish() {
    let mut a = PpuThreadTable::new();
    let mut b = PpuThreadTable::new();
    let id_a = a.create(UnitId::new(1), dummy_attrs()).unwrap();
    let id_b = b.create(UnitId::new(1), dummy_attrs()).unwrap();
    assert_eq!(a.state_hash(), b.state_hash());
    a.mark_finished(id_a, 42);
    assert_ne!(a.state_hash(), b.state_hash());
    b.mark_finished(id_b, 42);
    assert_eq!(a.state_hash(), b.state_hash());
}

#[test]
fn state_hash_folds_tls_base() {
    let mut a = PpuThreadTable::new();
    let mut b = PpuThreadTable::new();
    let mut attrs_a = dummy_attrs();
    let mut attrs_b = dummy_attrs();
    attrs_a.tls_base = 0x0020_0000;
    attrs_b.tls_base = 0x0030_0000;
    a.create(UnitId::new(1), attrs_a).unwrap();
    b.create(UnitId::new(1), attrs_b).unwrap();
    assert_ne!(a.state_hash(), b.state_hash());
}

#[test]
fn state_hash_join_waiter_list_length_is_load_bearing() {
    // Without a length prefix, ([X,Y], []) collides with
    // ([X], [Y]).
    let mut a = PpuThreadTable::new();
    let mut b = PpuThreadTable::new();
    a.insert_primary(UnitId::new(1), dummy_attrs());
    b.insert_primary(UnitId::new(1), dummy_attrs());
    let a_child1 = a.create(UnitId::new(2), dummy_attrs()).unwrap();
    let _a_child2 = a.create(UnitId::new(3), dummy_attrs()).unwrap();
    let b_child1 = b.create(UnitId::new(2), dummy_attrs()).unwrap();
    let b_child2 = b.create(UnitId::new(3), dummy_attrs()).unwrap();
    a.add_join_waiter(a_child1, PpuThreadId::new(0x42));
    a.add_join_waiter(a_child1, PpuThreadId::new(0x43));
    b.add_join_waiter(b_child1, PpuThreadId::new(0x42));
    b.add_join_waiter(b_child2, PpuThreadId::new(0x43));
    assert_ne!(a.state_hash(), b.state_hash());
}

#[test]
fn alias_unit_routes_unit_id_to_existing_primary() {
    let mut t = PpuThreadTable::new();
    t.insert_primary(UnitId::new(0), dummy_attrs());
    assert!(t.alias_unit(UnitId::new(7), PpuThreadId::PRIMARY));
    assert_eq!(
        t.thread_id_for_unit(UnitId::new(7)),
        Some(PpuThreadId::PRIMARY),
    );
    // Underlying thread record is unchanged.
    let p = t.get(PpuThreadId::PRIMARY).unwrap();
    assert_eq!(p.unit_id, UnitId::new(0));
}

#[test]
fn alias_unit_rejects_unknown_target_thread() {
    let mut t = PpuThreadTable::new();
    assert!(!t.alias_unit(UnitId::new(7), PpuThreadId::PRIMARY));
    assert!(t.thread_id_for_unit(UnitId::new(7)).is_none());
}

#[test]
fn alias_unit_rejects_already_mapped_unit() {
    let mut t = PpuThreadTable::new();
    t.insert_primary(UnitId::new(0), dummy_attrs());
    assert!(!t.alias_unit(UnitId::new(0), PpuThreadId::PRIMARY));
}

#[test]
fn drop_alias_restores_strict_esrch_for_unit() {
    let mut t = PpuThreadTable::new();
    t.insert_primary(UnitId::new(0), dummy_attrs());
    assert!(t.alias_unit(UnitId::new(7), PpuThreadId::PRIMARY));
    assert!(t.drop_alias(UnitId::new(7)));
    assert!(t.thread_id_for_unit(UnitId::new(7)).is_none());
    // Drop is idempotent at the no-op shape: second call returns false.
    assert!(!t.drop_alias(UnitId::new(7)));
}

#[test]
fn iter_ids_returns_deterministic_order() {
    let mut t = PpuThreadTable::new();
    t.insert_primary(UnitId::new(1), dummy_attrs());
    t.create(UnitId::new(2), dummy_attrs()).unwrap();
    t.create(UnitId::new(3), dummy_attrs()).unwrap();
    let ids: Vec<_> = t.iter_ids().collect();
    assert_eq!(ids.len(), 3);
    assert_eq!(ids[0], PpuThreadId::PRIMARY);
    assert_eq!(ids[1].raw(), 0x0100_0001);
    assert_eq!(ids[2].raw(), 0x0100_0002);
    let ids2: Vec<_> = t.iter_ids().collect();
    assert_eq!(ids, ids2);
}
