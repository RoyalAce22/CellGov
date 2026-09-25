//! Structural checks of the Multilinear-128 unit-status hash and of the
//! accumulator the registry keeps for it.

use super::*;
use crate::registry::test_fixtures::status_unit;
use crate::registry::UnitRegistry;

const ALL: [UnitStatus; 4] = [
    UnitStatus::Runnable,
    UnitStatus::Blocked,
    UnitStatus::Faulted,
    UnitStatus::Finished,
];

/// `key(S, 0) >> 64`: the hash of an all-zero lane vector.
#[test]
fn empty_registry_hashes_to_the_additive_key() {
    assert_eq!(UnitRegistry::new().status_hash(), 0x39D1_9DE9_68E7_3B36);
    assert_eq!(
        UnitRegistry::new().status_hash(),
        (status_key(0) >> 64) as u64
    );
}

#[test]
fn a_registered_runnable_unit_differs_from_an_absent_one() {
    let mut r = UnitRegistry::new();
    let (_h, f) = status_unit(UnitStatus::Runnable);
    r.register_with(f);
    assert_eq!(status_lane(Some(UnitStatus::Runnable)), 1);
    assert_eq!(status_lane(None), 0);
    assert_ne!(r.status_hash(), UnitRegistry::new().status_hash());
}

#[test]
fn every_single_lane_change_moves_the_hash() {
    for from in ALL {
        for to in ALL {
            if from == to {
                continue;
            }
            let mut r = UnitRegistry::new();
            let (_h0, f0) = status_unit(UnitStatus::Blocked);
            r.register_with(f0);
            let (handle, f1) = status_unit(from);
            let id = r.register_with(f1);
            let before = r.status_hash();
            handle.set(to);
            r.get_mut(id);
            assert_ne!(r.status_hash(), before, "{from:?} -> {to:?}");
        }
    }
    for status in ALL {
        let mut r = UnitRegistry::new();
        let before = r.status_hash();
        let (_h, f) = status_unit(status);
        r.register_with(f);
        assert_ne!(r.status_hash(), before, "register {status:?}");
    }
}

#[test]
fn two_units_that_exchange_statuses_hash_differently() {
    fn pair(a: UnitStatus, b: UnitStatus) -> u64 {
        let mut r = UnitRegistry::new();
        let (_ha, fa) = status_unit(a);
        let (_hb, fb) = status_unit(b);
        r.register_with(fa);
        r.register_with(fb);
        r.status_hash()
    }
    for a in ALL {
        for b in ALL {
            if a != b {
                assert_ne!(pair(a, b), pair(b, a), "{a:?} / {b:?}");
            }
        }
    }
}

/// Every path that changes an effective status keeps the accumulator
/// equal to a rebuild from every unit, in both build profiles.
#[test]
fn the_accumulator_tracks_every_status_path() {
    let mut r = UnitRegistry::new();
    let (h0, f0) = status_unit(UnitStatus::Runnable);
    let (_h1, f1) = status_unit(UnitStatus::Blocked);
    let a = r.register_with(f0);
    let b = r.register_with(f1);
    let check = |r: &UnitRegistry| assert_eq!(r.status_hash(), r.status_hash_from_scratch());
    check(&r);

    h0.set(UnitStatus::Faulted);
    r.get_mut(a);
    check(&r);

    h0.set(UnitStatus::Finished);
    for _ in r.iter_mut() {}
    check(&r);

    r.set_status_override(b, UnitStatus::Runnable);
    check(&r);
    r.clear_status_override(b);
    check(&r);

    let (_h2, f2) = status_unit(UnitStatus::Runnable);
    r.register_with(f2);
    check(&r);

    let copy = r.clone();
    check(&copy);
    assert_eq!(copy.status_hash(), r.status_hash());
}
