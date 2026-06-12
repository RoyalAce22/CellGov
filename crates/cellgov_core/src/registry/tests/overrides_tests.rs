//! Status-override precedence over unit self-reported status, including set and clear.

use super::*;
use crate::registry::test_fixtures::status_unit;

#[test]
fn effective_status_returns_unit_self_report_by_default() {
    let mut r = UnitRegistry::new();
    let (handle, factory) = status_unit(UnitStatus::Runnable);
    let id = r.register_with(factory);
    assert_eq!(r.effective_status(id), Some(UnitStatus::Runnable));
    handle.set(UnitStatus::Finished);
    assert_eq!(r.effective_status(id), Some(UnitStatus::Finished));
}

#[test]
fn set_status_override_takes_precedence_over_unit() {
    let mut r = UnitRegistry::new();
    let (_handle, factory) = status_unit(UnitStatus::Runnable);
    let id = r.register_with(factory);
    r.set_status_override(id, UnitStatus::Blocked);
    assert_eq!(r.effective_status(id), Some(UnitStatus::Blocked));
}

#[test]
fn clear_status_override_restores_unit_self_report() {
    let mut r = UnitRegistry::new();
    let (_handle, factory) = status_unit(UnitStatus::Runnable);
    let id = r.register_with(factory);
    r.set_status_override(id, UnitStatus::Blocked);
    assert_eq!(r.effective_status(id), Some(UnitStatus::Blocked));
    r.clear_status_override(id);
    assert_eq!(r.effective_status(id), Some(UnitStatus::Runnable));
}

#[test]
fn set_status_override_on_unknown_id_is_noop() {
    let mut r = UnitRegistry::new();
    r.set_status_override(UnitId::new(99), UnitStatus::Blocked);
    assert!(r.effective_status(UnitId::new(99)).is_none());
}
