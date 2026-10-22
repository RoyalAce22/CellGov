//! A finished group starts again, whether or not a join has read it: the
//! second run's SPUs register under the same thread ids, the join record
//! goes, and each thread keeps its previous exit status until it exits
//! again.

use super::*;
use crate::image::SpuImageHandle;
use crate::thread_group::{GroupState, MAX_SLOTS_PER_GROUP};

/// A one-thread group whose thread exited with `status`.
fn finished_group(status: u32) -> (ThreadGroupTable, u32) {
    let mut t = ThreadGroupTable::new();
    let gid = t.create(1).unwrap();
    t.initialize_thread(gid, 0, SpuImageHandle::new(1).unwrap(), [0; 4])
        .unwrap();
    t.get_mut(gid).unwrap().state = GroupState::Running;
    t.record_spu(UnitId::new(10), gid, 0).unwrap();
    assert_eq!(t.thread_exit(UnitId::new(10), status), Ok(Some(gid)));
    (t, gid)
}

#[test]
fn a_running_group_does_not_reset() {
    let mut t = ThreadGroupTable::new();
    let gid = t.create(1).unwrap();
    t.get_mut(gid).unwrap().state = GroupState::Running;
    assert!(!t.reset_for_restart(gid));
    assert_eq!(t.get(gid).unwrap().state, GroupState::Running);
}

#[test]
fn a_finished_group_resets_to_created_and_registers_a_new_spu() {
    let (mut t, gid) = finished_group(7);
    let thread = gid * MAX_SLOTS_PER_GROUP;
    assert!(t.reset_for_restart(gid));
    let group = t.get(gid).unwrap();
    assert_eq!(group.state, GroupState::Created);
    assert_eq!(group.slots.len(), 1, "the slots stay");

    t.get_mut(gid).unwrap().state = GroupState::Running;
    t.record_spu(UnitId::new(11), gid, 0).unwrap();
    assert_eq!(t.unit_for_thread(thread), Some(UnitId::new(11)));
    assert_eq!(t.group_of(UnitId::new(10)), None, "the old unit is gone");
}

#[test]
fn a_restarted_thread_reports_its_previous_exit_status_until_it_exits_again() {
    let (mut t, gid) = finished_group(7);
    let thread = gid * MAX_SLOTS_PER_GROUP;
    assert!(t.reset_for_restart(gid));
    assert_eq!(t.thread_exit_status(thread), Ok(7), "before the new run");
    t.get_mut(gid).unwrap().state = GroupState::Running;
    t.record_spu(UnitId::new(11), gid, 0).unwrap();
    assert_eq!(
        t.thread_exit_status(thread),
        Ok(7),
        "while the new run runs"
    );
    assert_eq!(t.thread_exit(UnitId::new(11), 9), Ok(Some(gid)));
    assert_eq!(
        t.thread_exit_status(thread),
        Ok(9),
        "the new exit replaces it"
    );
}

#[test]
fn the_suspend_mark_and_a_carried_exit_status_reach_the_sync_hash() {
    let (mut t, gid) = finished_group(7);
    let before = t.sync_partial();
    assert!(t.reset_for_restart(gid));
    assert_ne!(t.sync_partial(), before);
    assert_eq!(t.sync_partial(), t.sync_partial_from_scratch());

    let mut u = ThreadGroupTable::new();
    let g = u.create(1).unwrap();
    u.get_mut(g).unwrap().state = GroupState::Running;
    let running = u.sync_partial();
    assert!(u.suspend(g).is_ok());
    assert_ne!(u.sync_partial(), running);
    assert_eq!(u.sync_partial(), u.sync_partial_from_scratch());
}
