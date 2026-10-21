//! A unit's clock: where it starts, and the tick a woken unit resumes at.

use super::*;
use crate::registry::test_fixtures::{status_unit, StatusHandle};

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
fn a_unit_starts_at_the_tick_of_the_first_sync_that_sees_it() {
    let mut r = UnitRegistry::new();
    let (_handle, factory) = status_unit(UnitStatus::Runnable);
    let id = r.register_with(factory);
    assert_eq!(r.clock(id), GuestTicks::ZERO);
    r.sync_clocks(at(500));
    assert_eq!(r.clock(id), at(500));
    r.sync_clocks(at(900));
    assert_eq!(r.clock(id), at(500), "a running unit keeps its own clock");
}

#[test]
fn a_parked_unit_resumes_at_the_tick_that_woke_it() {
    let mut r = UnitRegistry::new();
    let (handle, factory) = status_unit(UnitStatus::Runnable);
    let id = r.register_with(factory);
    r.sync_clocks(at(100));
    set_status(&mut r, &handle, id, UnitStatus::Blocked);
    r.sync_clocks(at(200));
    assert_eq!(r.clock(id), at(100), "a parked unit's clock waits");
    set_status(&mut r, &handle, id, UnitStatus::Runnable);
    r.sync_clocks(at(700));
    assert_eq!(r.clock(id), at(700));
}

#[test]
fn a_wake_never_moves_a_clock_backwards() {
    let mut r = UnitRegistry::new();
    let (handle, factory) = status_unit(UnitStatus::Runnable);
    let id = r.register_with(factory);
    r.sync_clocks(at(0));
    r.set_clock(id, at(900));
    set_status(&mut r, &handle, id, UnitStatus::Blocked);
    r.sync_clocks(at(900));
    set_status(&mut r, &handle, id, UnitStatus::Runnable);
    r.sync_clocks(at(400));
    assert_eq!(r.clock(id), at(900));
}

#[test]
fn a_unit_held_for_its_start_resumes_at_the_start() {
    let mut r = UnitRegistry::new();
    let (_handle, factory) = status_unit(UnitStatus::Runnable);
    let id = r.register_with(factory);
    r.set_suspended(id, true);
    r.sync_clocks(at(50));
    assert_eq!(r.clock(id), at(50));
    r.set_suspended(id, false);
    r.sync_clocks(at(3_000));
    assert_eq!(r.clock(id), at(3_000));
}

#[test]
fn a_stopped_unit_resumes_at_the_tick_that_restarted_it() {
    let mut r = UnitRegistry::new();
    let (handle, factory) = status_unit(UnitStatus::Finished);
    let id = r.register_with(factory);
    r.sync_clocks(at(100));
    set_status(&mut r, &handle, id, UnitStatus::Runnable);
    r.sync_clocks(at(4_000));
    assert_eq!(r.clock(id), at(4_000));
    assert_eq!(r.lowest_runnable(), Some((at(4_000), id)));
}

#[test]
fn the_lowest_runnable_unit_follows_each_step_s_clock() {
    let mut r = UnitRegistry::new();
    let (_a, first) = status_unit(UnitStatus::Runnable);
    let (_b, second) = status_unit(UnitStatus::Runnable);
    let first = r.register_with(first);
    let second = r.register_with(second);
    r.sync_clocks(at(0));
    assert_eq!(
        r.lowest_runnable(),
        Some((at(0), first)),
        "a tie goes to the lower id"
    );
    r.set_clock(first, at(50));
    assert_eq!(r.lowest_runnable(), Some((at(0), second)));
    r.set_clock(second, at(70));
    assert_eq!(r.lowest_runnable(), Some((at(50), first)));
}

#[test]
fn a_runnable_unit_is_not_raised_by_a_later_sync() {
    let mut r = UnitRegistry::new();
    let (_handle, factory) = status_unit(UnitStatus::Runnable);
    let id = r.register_with(factory);
    r.sync_clocks(at(10));
    r.sync_clocks(at(10_000));
    assert_eq!(r.clock(id), at(10));
}
