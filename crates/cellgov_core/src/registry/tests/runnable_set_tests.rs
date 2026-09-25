//! Structural checks of the Multilinear-128 runnable-set hash and of its
//! accumulator.

use super::*;
use crate::registry::test_fixtures::status_unit;
use crate::registry::UnitRegistry;

const NOT_RUNNABLE: [UnitStatus; 3] = [
    UnitStatus::Blocked,
    UnitStatus::Faulted,
    UnitStatus::Finished,
];

/// `key(R, 0) >> 64`: the hash of the empty runnable set.
#[test]
fn an_empty_runnable_set_hashes_to_the_additive_key() {
    assert_eq!(
        UnitRegistry::new().runnable_queue_hash(),
        0x355F_A152_B0A0_34EF
    );
    assert_eq!(
        UnitRegistry::new().runnable_queue_hash(),
        (runnable_key(0) >> 64) as u64
    );
}

#[test]
fn entering_or_leaving_runnable_moves_the_hash() {
    for other in NOT_RUNNABLE {
        let mut r = UnitRegistry::new();
        let (_h0, f0) = status_unit(UnitStatus::Runnable);
        r.register_with(f0);
        let (handle, f1) = status_unit(UnitStatus::Runnable);
        let id = r.register_with(f1);
        let runnable = r.runnable_queue_hash();
        handle.set(other);
        r.get_mut(id);
        let left = r.runnable_queue_hash();
        assert_ne!(left, runnable, "Runnable -> {other:?}");
        handle.set(UnitStatus::Runnable);
        r.get_mut(id);
        assert_eq!(r.runnable_queue_hash(), runnable, "{other:?} -> Runnable");
    }
}

/// Membership alone: a unit moving between two non-Runnable statuses
/// leaves the set, and so the hash, as it was.
#[test]
fn a_change_between_two_non_runnable_statuses_keeps_the_hash() {
    let mut r = UnitRegistry::new();
    let (handle, f) = status_unit(UnitStatus::Blocked);
    let id = r.register_with(f);
    let before = r.runnable_queue_hash();
    handle.set(UnitStatus::Faulted);
    r.get_mut(id);
    assert_eq!(r.runnable_queue_hash(), before);
    assert_ne!(r.status_hash(), {
        let mut fresh = UnitRegistry::new();
        let (_h, f) = status_unit(UnitStatus::Blocked);
        fresh.register_with(f);
        fresh.status_hash()
    });
}

/// The order in which units enter and leave the set does not matter,
/// only the set they end in.
#[test]
fn the_hash_is_independent_of_the_order_of_changes() {
    fn registry() -> (
        UnitRegistry,
        Vec<(UnitId, crate::registry::test_fixtures::StatusHandle)>,
    ) {
        let mut r = UnitRegistry::new();
        let mut handles = Vec::new();
        for _ in 0..3 {
            let (h, f) = status_unit(UnitStatus::Blocked);
            handles.push((r.register_with(f), h));
        }
        (r, handles)
    }
    let (mut a, ha) = registry();
    let (mut b, hb) = registry();
    for i in [0, 2] {
        ha[i].1.set(UnitStatus::Runnable);
        a.get_mut(ha[i].0);
        a.runnable_queue_hash();
    }
    for i in [2, 1, 0] {
        hb[i].1.set(UnitStatus::Runnable);
        b.get_mut(hb[i].0);
        b.runnable_queue_hash();
    }
    hb[1].1.set(UnitStatus::Blocked);
    b.get_mut(hb[1].0);
    assert_eq!(a.runnable_queue_hash(), b.runnable_queue_hash());
}

#[test]
fn two_sets_that_differ_by_one_unit_hash_differently() {
    fn set(runnable: &[bool]) -> u64 {
        let mut r = UnitRegistry::new();
        for &on in runnable {
            let status = if on {
                UnitStatus::Runnable
            } else {
                UnitStatus::Blocked
            };
            let (_h, f) = status_unit(status);
            r.register_with(f);
        }
        r.runnable_queue_hash()
    }
    assert_ne!(set(&[true, false, true]), set(&[true, false, false]));
    assert_ne!(set(&[true, false, true]), set(&[false, false, true]));
    assert_ne!(set(&[true, false, false]), set(&[false, true, false]));
}

/// Every path that changes an effective status keeps the runnable-set
/// accumulator equal to a rebuild, in both build profiles.
#[test]
fn the_runnable_accumulator_tracks_every_status_path() {
    let mut r = UnitRegistry::new();
    let (h0, f0) = status_unit(UnitStatus::Runnable);
    let (_h1, f1) = status_unit(UnitStatus::Blocked);
    let a = r.register_with(f0);
    let b = r.register_with(f1);
    let check = |r: &UnitRegistry| {
        assert_eq!(
            r.runnable_queue_hash(),
            r.runnable_queue_hash_from_scratch()
        )
    };
    check(&r);

    h0.set(UnitStatus::Faulted);
    r.get_mut(a);
    check(&r);

    h0.set(UnitStatus::Runnable);
    for _ in r.iter_mut() {}
    check(&r);

    r.set_status_override(b, UnitStatus::Runnable);
    check(&r);
    r.set_status_override(a, UnitStatus::Blocked);
    check(&r);
    r.clear_status_override(b);
    check(&r);

    let (_h2, f2) = status_unit(UnitStatus::Runnable);
    r.register_with(f2);
    check(&r);

    let copy = r.clone();
    check(&copy);
    assert_eq!(copy.runnable_queue_hash(), r.runnable_queue_hash());
}
