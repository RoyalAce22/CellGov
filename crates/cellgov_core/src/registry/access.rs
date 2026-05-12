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

    /// Count the units whose effective status is `Runnable`.
    ///
    /// Respects status overrides. The scheduler uses the count to
    /// short-circuit `AllBlocked` (zero) and single-runnable (one)
    /// cases without walking the rotation.
    pub fn count_runnable(&self) -> usize {
        if self.status_overrides.is_empty() {
            return self
                .units
                .values()
                .filter(|u| u.status() == UnitStatus::Runnable)
                .count();
        }
        self.runnable_ids().count()
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_fixtures::{status_unit, CountingUnit};
    use super::*;
    use cellgov_exec::ExecutionContext;
    use cellgov_mem::GuestMemory;
    use cellgov_time::{Budget, InstructionCost};

    #[test]
    fn new_is_empty() {
        let r = UnitRegistry::new();
        assert!(r.is_empty());
        assert_eq!(r.len(), 0);
        assert_eq!(r.ids().count(), 0);
    }

    #[test]
    fn get_returns_registered_unit() {
        let mut r = UnitRegistry::new();
        let id = r.register_with(|id| CountingUnit { id, steps: 0 });
        let u = r.get(id).expect("present");
        assert_eq!(u.unit_id(), id);
        assert_eq!(u.status(), UnitStatus::Runnable);
    }

    #[test]
    fn get_missing_is_none() {
        let r = UnitRegistry::new();
        assert!(r.get(UnitId::new(99)).is_none());
    }

    #[test]
    fn get_mut_drives_run_until_yield() {
        let mut r = UnitRegistry::new();
        let id = r.register_with(|id| CountingUnit { id, steps: 0 });
        let mem = GuestMemory::new(8);
        let ctx = ExecutionContext::new(&mem);
        let u = r.get_mut(id).expect("present");
        let mut effects = Vec::new();
        let step = u.run_until_yield(Budget::new(5), &ctx, &mut effects);
        assert_eq!(step.consumed_cost, InstructionCost::new(5));
        assert_eq!(effects.len(), 1);
    }

    #[test]
    fn iter_is_in_id_order() {
        let mut r = UnitRegistry::new();
        for _ in 0..4 {
            r.register_with(|id| CountingUnit { id, steps: 0 });
        }
        let ids: Vec<u64> = r.iter().map(|(id, _)| id.raw()).collect();
        assert_eq!(ids, vec![0, 1, 2, 3]);
    }

    #[test]
    fn ids_iterator_matches_registration_order() {
        let mut r = UnitRegistry::new();
        for _ in 0..3 {
            r.register_with(|id| CountingUnit { id, steps: 0 });
        }
        let collected: Vec<UnitId> = r.ids().collect();
        assert_eq!(
            collected,
            vec![UnitId::new(0), UnitId::new(1), UnitId::new(2)]
        );
    }

    #[test]
    fn iter_mut_can_step_every_unit() {
        let mut r = UnitRegistry::new();
        for _ in 0..3 {
            r.register_with(|id| CountingUnit { id, steps: 0 });
        }
        let mem = GuestMemory::new(8);
        let ctx = ExecutionContext::new(&mem);
        let mut total = 0;
        let mut effects = Vec::new();
        for (_, u) in r.iter_mut() {
            effects.clear();
            u.run_until_yield(Budget::new(1), &ctx, &mut effects);
            total += effects.len();
        }
        assert_eq!(total, 3);
    }

    #[test]
    fn count_runnable_matches_runnable_ids() {
        let mut r = UnitRegistry::new();
        let (h0, f0) = status_unit(UnitStatus::Runnable);
        let (h1, f1) = status_unit(UnitStatus::Blocked);
        let (h2, f2) = status_unit(UnitStatus::Runnable);
        r.register_with(f0);
        r.register_with(f1);
        r.register_with(f2);
        assert_eq!(r.count_runnable(), 2);
        assert_eq!(r.runnable_ids().count(), 2);
        r.set_status_override(UnitId::new(0), UnitStatus::Blocked);
        assert_eq!(r.count_runnable(), 1);
        h1.set(UnitStatus::Runnable);
        assert_eq!(r.count_runnable(), 2);
        r.clear_status_override(UnitId::new(0));
        assert_eq!(r.count_runnable(), 3);
        let _ = (h0, h2);
    }

    #[test]
    fn count_runnable_empty_registry_is_zero() {
        let r = UnitRegistry::new();
        assert_eq!(r.count_runnable(), 0);
    }

    #[test]
    fn count_runnable_all_blocked_is_zero() {
        let mut r = UnitRegistry::new();
        for _ in 0..3 {
            let (_h, f) = status_unit(UnitStatus::Blocked);
            r.register_with(f);
        }
        assert_eq!(r.count_runnable(), 0);
    }
}
