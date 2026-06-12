//! Constructors and lifecycle: `new`, `len`/`is_empty`, and the two
//! registration entry points that allocate stable `UnitId`s.

use cellgov_event::UnitId;
use cellgov_exec::ExecutionUnit;

use super::{RegisteredUnit, UnitRegistry};

impl UnitRegistry {
    /// Construct an empty registry.
    #[inline]
    pub fn new() -> Self {
        Self::default()
    }

    /// Number of registered units.
    #[inline]
    pub fn len(&self) -> usize {
        self.units.len()
    }

    /// Whether the registry holds any units.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.units.is_empty()
    }

    /// Register a unit; `factory` receives the allocated id and must
    /// return a unit whose `unit_id()` equals that id.
    ///
    /// # Panics
    ///
    /// Panics if the constructed unit's `unit_id()` disagrees with the
    /// assigned id.
    ///
    /// `next_id` only advances on successful construction; a factory
    /// panic leaves the counter untouched so a caller that retries
    /// after catching the unwind reuses the same id. A hole in the id
    /// sequence would silently change replay hashes.
    pub fn register_with<U, F>(&mut self, factory: F) -> UnitId
    where
        U: ExecutionUnit + Clone + 'static,
        F: FnOnce(UnitId) -> U,
    {
        let id = UnitId::new(self.next_id);
        let unit = factory(id);
        assert_eq!(
            ExecutionUnit::unit_id(&unit),
            id,
            "registered unit reported {} but registry assigned {}",
            ExecutionUnit::unit_id(&unit).raw(),
            id.raw(),
        );
        self.next_id += 1;
        let prev = self.units.insert(id, Box::new(unit));
        debug_assert!(
            prev.is_none(),
            "UnitRegistry: next_id {id:?} already had a unit -- monotonic counter wrapped or a \
             future refactor started recycling ids; duplicate insert would silently drop the \
             old unit"
        );
        id
    }

    /// Register a unit produced by a boxed factory. Same id-allocation
    /// and factory-panic contract as [`Self::register_with`].
    pub fn register_dynamic(
        &mut self,
        factory: &dyn Fn(UnitId) -> Box<dyn RegisteredUnit>,
    ) -> UnitId {
        let id = UnitId::new(self.next_id);
        let unit = factory(id);
        assert_eq!(
            unit.unit_id(),
            id,
            "registered unit reported {} but registry assigned {}",
            unit.unit_id().raw(),
            id.raw(),
        );
        self.next_id += 1;
        let prev = self.units.insert(id, unit);
        debug_assert!(
            prev.is_none(),
            "UnitRegistry: next_id {id:?} already had a unit -- monotonic counter wrapped or a \
             future refactor started recycling ids; duplicate insert would silently drop the \
             old unit"
        );
        id
    }
}

#[cfg(test)]
#[path = "tests/registration_tests.rs"]
mod tests;
