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
    /// unit's self-reported `status()`; and [`UnitStatus::Suspended`]
    /// over a Runnable or Blocked status while the unit is suspended,
    /// and over a Runnable status while the unit waits for a PPE
    /// hardware thread.
    pub fn effective_status(&self, id: UnitId) -> Option<UnitStatus> {
        let status = self.status_beneath_hold(id)?;
        let held = (!self.suspended.is_empty()
            && matches!(status, UnitStatus::Runnable | UnitStatus::Blocked)
            && self.suspended.contains(&id))
            || (status == UnitStatus::Runnable && self.off_core.contains(&id));
        Some(if held { UnitStatus::Suspended } else { status })
    }

    /// Whether `id` is held by a suspension: its SPU thread group is
    /// suspended or its PPU thread waits for its start.
    pub(crate) fn is_suspended(&self, id: UnitId) -> bool {
        self.suspended.contains(&id)
    }

    /// Hold exactly the units of `off_core` out of the runnable set as
    /// PPU threads waiting for a PPE hardware thread, releasing every
    /// other.
    pub(crate) fn set_off_core(&mut self, off_core: &std::collections::BTreeSet<UnitId>) {
        if &self.off_core == off_core {
            return;
        }
        let changed: Vec<UnitId> = self
            .off_core
            .symmetric_difference(off_core)
            .copied()
            .collect();
        self.off_core.clone_from(off_core);
        for id in changed {
            if self.units.contains_key(&id) {
                self.mark_status(id);
            }
        }
    }

    /// Whether `id` is a runnable PPU thread waiting for a PPE hardware
    /// thread.
    pub fn is_off_core(&self, id: UnitId) -> bool {
        self.off_core.contains(&id) && self.status_beneath_hold(id) == Some(UnitStatus::Runnable)
    }

    /// Whether any PPU thread waits for a PPE hardware thread.
    pub(crate) fn any_off_core(&self) -> bool {
        self.off_core
            .iter()
            .any(|&id| self.status_beneath_hold(id) == Some(UnitStatus::Runnable))
    }

    /// The status of a unit beneath a suspension hold: the runtime
    /// override if set, else the unit's self-reported `status()`. A wake
    /// reads this, so a message or signal that lands while the unit's
    /// group is suspended still ends its park.
    pub(crate) fn status_beneath_hold(&self, id: UnitId) -> Option<UnitStatus> {
        let unit = self.units.get(&id)?;
        Some(
            self.status_overrides
                .get(&id)
                .copied()
                .unwrap_or_else(|| unit.status()),
        )
    }

    /// Hold `id` out of the runnable set while its SPU thread group is
    /// suspended or its PPU thread waits for its start (`true`), or
    /// release it (`false`). No-op for unknown ids.
    pub(crate) fn set_suspended(&mut self, id: UnitId, suspended: bool) {
        if !self.units.contains_key(&id) {
            return;
        }
        let changed = if suspended {
            self.suspended.insert(id)
        } else {
            self.suspended.remove(&id)
        };
        if changed {
            self.mark_status(id);
        }
    }

    /// The runtime-side override on `id`, if one is set.
    pub fn status_override(&self, id: UnitId) -> Option<UnitStatus> {
        self.status_overrides.get(&id).copied()
    }

    /// Set a runtime-side status override. No-op for unknown ids.
    pub fn set_status_override(&mut self, id: UnitId, status: UnitStatus) {
        if self.units.contains_key(&id) {
            self.status_overrides.insert(id, status);
            self.mark_status(id);
        }
    }

    /// Clear a runtime-side status override, if any. Called every step;
    /// the `is_empty()` guard avoids a `BTreeMap::remove` probe in the
    /// common case of no overrides.
    pub fn clear_status_override(&mut self, id: UnitId) {
        if self.status_overrides.is_empty() {
            return;
        }
        if self.status_overrides.remove(&id).is_some() {
            self.mark_status(id);
        }
    }
}

#[cfg(test)]
#[path = "tests/overrides_tests.rs"]
mod tests;
