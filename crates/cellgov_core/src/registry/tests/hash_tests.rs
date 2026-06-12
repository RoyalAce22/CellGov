//! Registry status-hash and runnable-queue-hash sensitivity, pinned by wire-format goldens.

use super::*;
use crate::registry::test_fixtures::status_unit;

#[test]
fn status_hash_of_empty_registry_is_stable() {
    let a = UnitRegistry::new();
    let b = UnitRegistry::new();
    assert_eq!(a.status_hash(), b.status_hash());
}

#[test]
fn status_hash_changes_when_a_unit_status_changes() {
    let mut r = UnitRegistry::new();
    let (handle, factory) = status_unit(UnitStatus::Runnable);
    r.register_with(factory);
    let h0 = r.status_hash();
    handle.set(UnitStatus::Blocked);
    let h1 = r.status_hash();
    handle.set(UnitStatus::Finished);
    let h2 = r.status_hash();
    assert_ne!(h0, h1);
    assert_ne!(h1, h2);
    assert_ne!(h0, h2);
}

#[test]
fn status_hash_distinguishes_each_status_variant() {
    fn one(s: UnitStatus) -> u64 {
        let mut r = UnitRegistry::new();
        let (_h, factory) = status_unit(s);
        r.register_with(factory);
        r.status_hash()
    }
    let all: std::collections::BTreeSet<u64> = [
        one(UnitStatus::Runnable),
        one(UnitStatus::Blocked),
        one(UnitStatus::Faulted),
        one(UnitStatus::Finished),
    ]
    .into_iter()
    .collect();
    assert_eq!(all.len(), 4);
}

#[test]
fn status_override_affects_status_hash() {
    let mut r = UnitRegistry::new();
    let (_handle, factory) = status_unit(UnitStatus::Runnable);
    let id = r.register_with(factory);
    let h_runnable = r.status_hash();
    r.set_status_override(id, UnitStatus::Blocked);
    let h_blocked = r.status_hash();
    assert_ne!(h_runnable, h_blocked);
    r.clear_status_override(id);
    assert_eq!(r.status_hash(), h_runnable);
}

#[test]
fn runnable_queue_hash_changes_when_unit_becomes_blocked() {
    let mut r = UnitRegistry::new();
    let (handle, factory) = status_unit(UnitStatus::Runnable);
    let _id = r.register_with(factory);
    let h_runnable = r.runnable_queue_hash();
    handle.set(UnitStatus::Blocked);
    let h_blocked = r.runnable_queue_hash();
    assert_ne!(h_runnable, h_blocked);
    handle.set(UnitStatus::Runnable);
    assert_eq!(r.runnable_queue_hash(), h_runnable);
}

#[test]
fn runnable_queue_hash_empty_when_no_runnable_units() {
    let mut r = UnitRegistry::new();
    let (_h, factory) = status_unit(UnitStatus::Finished);
    r.register_with(factory);
    let empty_reg = UnitRegistry::new();
    assert_eq!(r.runnable_queue_hash(), empty_reg.runnable_queue_hash());
}

#[test]
fn status_hash_is_id_position_sensitive() {
    let mut a = UnitRegistry::new();
    let (_ha, factory_a) = status_unit(UnitStatus::Runnable);
    a.register_with(factory_a);

    let mut b = UnitRegistry::new();
    let (_burn, burn_factory) = status_unit(UnitStatus::Finished);
    b.register_with(burn_factory);
    let (_hb, factory_b) = status_unit(UnitStatus::Runnable);
    b.register_with(factory_b);
    assert_ne!(a.status_hash(), b.status_hash());
}

/// Pins the `status_hash` wire format; catches reorders within
/// [`status_byte`] that the exhaustive match cannot.
#[test]
fn status_hash_wire_format_golden() {
    let mut r = UnitRegistry::new();
    let (_h0, f0) = status_unit(UnitStatus::Runnable);
    let (_h1, f1) = status_unit(UnitStatus::Blocked);
    let (_h2, f2) = status_unit(UnitStatus::Finished);
    r.register_with(f0);
    r.register_with(f1);
    r.register_with(f2);
    const EXPECTED_STATUS_HASH: u64 = 0xE465_5B46_398E_DE44;
    assert_eq!(
        r.status_hash(),
        EXPECTED_STATUS_HASH,
        "status_hash wire format drifted; if this change was \
         intentional, every existing trace is now incompatible"
    );
}

/// Pins the `runnable_queue_hash` wire format; catches drift in the
/// runnable-predicate shape that `status_byte` cannot.
#[test]
fn runnable_queue_hash_wire_format_golden() {
    let mut r = UnitRegistry::new();
    let (_h0, f0) = status_unit(UnitStatus::Runnable);
    let (_h1, f1) = status_unit(UnitStatus::Blocked);
    let (_h2, f2) = status_unit(UnitStatus::Runnable);
    let (_h3, f3) = status_unit(UnitStatus::Finished);
    r.register_with(f0);
    r.register_with(f1);
    r.register_with(f2);
    r.register_with(f3);
    const EXPECTED_RUNNABLE_QUEUE_HASH: u64 = 0xC615_ADCB_76DD_F8A7;
    assert_eq!(
        r.runnable_queue_hash(),
        EXPECTED_RUNNABLE_QUEUE_HASH,
        "runnable_queue_hash wire format drifted; if this change \
         was intentional, every existing trace is now incompatible"
    );
}
