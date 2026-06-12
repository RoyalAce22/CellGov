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
#[path = "tests/overrides_tests.rs"]
mod tests;
