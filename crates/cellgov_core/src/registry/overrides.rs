//! Runtime-side status overrides over per-unit self-reported state.
//!
//! Written by the commit pipeline, cleared at the start of each step.
//! Overrides take precedence over the unit's own `status()` for
//! scheduling and for [`UnitRegistry::status_hash`].

use cellgov_event::UnitId;
use cellgov_exec::UnitStatus;

use super::UnitRegistry;

impl UnitRegistry {
    /// Effective status of a unit: runtime override if set, else the
    /// unit's self-reported `status()`.
    pub fn effective_status(&self, id: UnitId) -> Option<UnitStatus> {
        let unit = self.units.get(&id)?;
        Some(
            self.status_overrides
                .get(&id)
                .copied()
                .unwrap_or_else(|| unit.status()),
        )
    }

    /// Set a runtime-side status override. No-op for unknown ids.
    pub fn set_status_override(&mut self, id: UnitId, status: UnitStatus) {
        if self.units.contains_key(&id) {
            self.status_overrides.insert(id, status);
        }
    }

    /// Clear a runtime-side status override, if any. Called every step;
    /// the `is_empty()` guard avoids a `BTreeMap::remove` probe in the
    /// common case of no overrides.
    pub fn clear_status_override(&mut self, id: UnitId) {
        if self.status_overrides.is_empty() {
            return;
        }
        self.status_overrides.remove(&id);
    }
}

#[cfg(test)]
mod tests {
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
}
