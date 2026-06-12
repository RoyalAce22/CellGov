//! Read-only and mutable accessors over the registered unit set, plus
//! the runnable-set predicates the scheduler consults.

use cellgov_event::UnitId;
use cellgov_exec::UnitStatus;

use super::{RegisteredUnit, UnitRegistry};

impl UnitRegistry {
    /// Borrow a unit by id, if present.
    #[inline]
    pub fn get(&self, id: UnitId) -> Option<&dyn RegisteredUnit> {
        self.units.get(&id).map(|u| u.as_ref())
    }

    /// Mutably borrow a unit by id, if present.
    #[inline]
    pub fn get_mut(&mut self, id: UnitId) -> Option<&mut dyn RegisteredUnit> {
        self.units.get_mut(&id).map(|u| u.as_mut())
    }

    /// Iterate registered units in id order.
    pub fn iter(&self) -> impl Iterator<Item = (UnitId, &dyn RegisteredUnit)> + '_ {
        self.units.iter().map(|(id, u)| (*id, u.as_ref()))
    }

    /// Iterate registered units mutably in id order.
    pub fn iter_mut(&mut self) -> impl Iterator<Item = (UnitId, &mut dyn RegisteredUnit)> + '_ {
        self.units.iter_mut().map(|(id, u)| (*id, u.as_mut()))
    }

    /// Iterate registered ids in id order.
    pub fn ids(&self) -> impl Iterator<Item = UnitId> + '_ {
        self.units.keys().copied()
    }

    /// Iterate unit ids whose effective status is `Runnable`.
    pub fn runnable_ids(&self) -> impl Iterator<Item = UnitId> + '_ {
        self.ids()
            .filter(move |id| self.effective_status(*id) == Some(UnitStatus::Runnable))
    }
}

#[cfg(test)]
#[path = "tests/access_tests.rs"]
mod tests;
