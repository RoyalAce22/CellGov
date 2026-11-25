//! The resume mark: a unit that a clock sync finds runnable after an
//! earlier sync found it parked.

use super::*;
use crate::registry::test_fixtures::{status_unit, StatusHandle};
use cellgov_exec::{ResumeKind, UnitStatus};
use cellgov_time::GuestTicks;
use std::collections::BTreeSet;

fn at(tick: u64) -> GuestTicks {
    GuestTicks::new(tick)
}

/// Change `id`'s status the way a real unit's does: through a mutable
/// borrow, which marks it for the next sync.
fn set_status(r: &mut UnitRegistry, handle: &StatusHandle, id: UnitId, status: UnitStatus) {
    handle.set(status);
    let _ = r.get_mut(id);
}

#[test]
fn a_woken_unit_is_marked_resumed_until_its_next_step_takes_the_mark() {
    let mut r = UnitRegistry::new();
    let (handle, factory) = status_unit(UnitStatus::Runnable);
    let id = r.register_with(factory);
    r.sync_clocks(at(100));
    set_status(&mut r, &handle, id, UnitStatus::Blocked);
    r.sync_clocks(at(200));
    assert_eq!(r.take_resumed(id), None, "a parked unit has not resumed");
    set_status(&mut r, &handle, id, UnitStatus::Runnable);
    r.sync_clocks(at(700));
    assert_eq!(r.take_resumed(id), Some(ResumeKind::Woken));
    assert_eq!(r.take_resumed(id), None, "the step took the mark");
}

#[test]
fn a_unit_that_stays_runnable_is_never_marked() {
    let mut r = UnitRegistry::new();
    let (_handle, factory) = status_unit(UnitStatus::Runnable);
    let id = r.register_with(factory);
    r.sync_clocks(at(10));
    let _ = r.get_mut(id);
    r.sync_clocks(at(20));
    assert_eq!(r.take_resumed(id), None);
}

#[test]
fn a_unit_back_on_a_hardware_thread_is_marked_preempted() {
    let mut r = UnitRegistry::new();
    let (_handle, factory) = status_unit(UnitStatus::Runnable);
    let id = r.register_with(factory);
    r.sync_clocks(at(0));
    r.set_off_core(&BTreeSet::from([id]));
    r.sync_clocks(at(50));
    assert_eq!(
        r.take_resumed(id),
        None,
        "a unit waiting for a hardware thread"
    );
    r.set_off_core(&BTreeSet::new());
    r.sync_clocks(at(900));
    assert_eq!(r.take_resumed(id), Some(ResumeKind::Preempted));
}

#[test]
fn a_woken_unit_that_then_waits_for_a_hardware_thread_is_marked_woken() {
    let mut r = UnitRegistry::new();
    let (handle, factory) = status_unit(UnitStatus::Runnable);
    let id = r.register_with(factory);
    r.sync_clocks(at(0));
    set_status(&mut r, &handle, id, UnitStatus::Blocked);
    r.sync_clocks(at(100));
    set_status(&mut r, &handle, id, UnitStatus::Runnable);
    r.set_off_core(&BTreeSet::from([id]));
    r.sync_clocks(at(200));
    assert_eq!(r.take_resumed(id), None, "woken, still off the PPE");
    r.set_off_core(&BTreeSet::new());
    r.sync_clocks(at(300));
    assert_eq!(r.take_resumed(id), Some(ResumeKind::Woken));
}

#[test]
fn a_wake_synced_before_the_unit_waits_for_a_hardware_thread_stays_woken() {
    let mut r = UnitRegistry::new();
    let (handle, factory) = status_unit(UnitStatus::Runnable);
    let id = r.register_with(factory);
    r.sync_clocks(at(0));
    set_status(&mut r, &handle, id, UnitStatus::Blocked);
    r.sync_clocks(at(100));
    // A commit syncs the wake, then places the PPE threads and syncs again.
    set_status(&mut r, &handle, id, UnitStatus::Runnable);
    r.sync_clocks(at(200));
    r.set_off_core(&BTreeSet::from([id]));
    r.sync_clocks(at(200));
    r.set_off_core(&BTreeSet::new());
    r.sync_clocks(at(300));
    assert_eq!(r.take_resumed(id), Some(ResumeKind::Woken));
}

#[test]
fn a_preempted_unit_marks_preempted_only_once() {
    let mut r = UnitRegistry::new();
    let (handle, factory) = status_unit(UnitStatus::Runnable);
    let id = r.register_with(factory);
    r.sync_clocks(at(0));
    r.set_off_core(&BTreeSet::from([id]));
    r.sync_clocks(at(50));
    r.set_off_core(&BTreeSet::new());
    r.sync_clocks(at(100));
    assert_eq!(r.take_resumed(id), Some(ResumeKind::Preempted));
    set_status(&mut r, &handle, id, UnitStatus::Blocked);
    r.sync_clocks(at(200));
    set_status(&mut r, &handle, id, UnitStatus::Runnable);
    r.sync_clocks(at(300));
    assert_eq!(
        r.take_resumed(id),
        Some(ResumeKind::Woken),
        "a later block resumes woken"
    );
}

#[test]
fn a_unit_that_blocks_while_it_waits_for_a_hardware_thread_resumes_woken() {
    let mut r = UnitRegistry::new();
    let (handle, factory) = status_unit(UnitStatus::Runnable);
    let id = r.register_with(factory);
    r.sync_clocks(at(0));
    r.set_off_core(&BTreeSet::from([id]));
    r.sync_clocks(at(50));
    set_status(&mut r, &handle, id, UnitStatus::Blocked);
    r.sync_clocks(at(100));
    r.set_off_core(&BTreeSet::new());
    set_status(&mut r, &handle, id, UnitStatus::Runnable);
    r.sync_clocks(at(200));
    assert_eq!(r.take_resumed(id), Some(ResumeKind::Woken));
}
