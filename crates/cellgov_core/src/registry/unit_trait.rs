//! Object-safe view of [`ExecutionUnit`] for the runtime registry.
//! [`ExecutionUnit`] has an associated `Snapshot` so it is not
//! object-safe; [`RegisteredUnit`] mirrors the runtime-visible methods
//! and is blanket-impl'd for every `U: ExecutionUnit + Clone + 'static`.

use core::any::Any;

use cellgov_effects::Effect;
use cellgov_event::UnitId;
use cellgov_exec::{
    ExecutionContext, ExecutionStepResult, ExecutionUnit, FaultRegisterDump, UnitStatus,
};
use cellgov_time::Budget;

/// Object-safe view of an execution unit.
///
/// Blanket-impl'd for every `U: ExecutionUnit + Clone + 'static`.
/// Method contracts mirror [`ExecutionUnit`]; see that trait for
/// the authoritative docs.
pub trait RegisteredUnit: 'static {
    /// Stable id assigned at registration.
    fn unit_id(&self) -> UnitId;

    /// Deep-clone behind a fresh box. Used by [`super::UnitRegistry::clone`]
    /// (and thus [`crate::Runtime::snapshot`]) to fork per-unit
    /// state without aliasing.
    fn clone_box(&self) -> Box<dyn RegisteredUnit>;

    /// Coarse runnability state queried by the scheduler.
    fn status(&self) -> UnitStatus;

    /// Run the unit until it yields.
    fn run_until_yield(
        &mut self,
        budget: Budget,
        ctx: &ExecutionContext<'_>,
        effects: &mut Vec<Effect>,
    ) -> ExecutionStepResult;

    /// Drain per-instruction state fingerprints retired during the most
    /// recent `run_until_yield`. See [`ExecutionUnit::drain_retired_state_hashes`].
    fn drain_retired_state_hashes(&mut self) -> Vec<(u64, u64)>;

    /// Drain full-register snapshots collected inside the zoom-in window.
    /// See [`ExecutionUnit::drain_retired_state_full`].
    fn drain_retired_state_full(&mut self) -> Vec<(u64, [u64; 32], u64, u64, u64, u32)>;

    /// Drain instruction-variant profiling data.
    /// See [`ExecutionUnit::drain_profile_insns`].
    fn drain_profile_insns(&mut self) -> Vec<(&'static str, u64)>;

    /// Drain adjacent-pair profiling data.
    /// See [`ExecutionUnit::drain_profile_pairs`].
    fn drain_profile_pairs(&mut self) -> Vec<((&'static str, &'static str), u64)>;

    /// Notify the unit that guest memory in `[addr, addr+len)` was written.
    /// See [`ExecutionUnit::invalidate_code`] for the must-override contract.
    fn invalidate_code(&mut self, addr: u64, len: u64);

    /// Shadow hit/miss counters. See [`ExecutionUnit::shadow_stats`].
    fn shadow_stats(&self) -> (u64, u64);

    /// Current register snapshot for diagnostic dumps. See
    /// [`ExecutionUnit::register_dump`].
    fn register_dump(&self) -> Option<FaultRegisterDump>;

    /// Upcast for callers that need to downcast to a concrete unit
    /// type to inspect state the trait does not expose.
    fn as_any(&self) -> &dyn Any;
}

impl<U: ExecutionUnit + Clone + 'static> RegisteredUnit for U {
    #[inline]
    fn unit_id(&self) -> UnitId {
        ExecutionUnit::unit_id(self)
    }

    #[inline]
    fn clone_box(&self) -> Box<dyn RegisteredUnit> {
        Box::new(self.clone())
    }

    #[inline]
    fn status(&self) -> UnitStatus {
        ExecutionUnit::status(self)
    }

    #[inline]
    fn run_until_yield(
        &mut self,
        budget: Budget,
        ctx: &ExecutionContext<'_>,
        effects: &mut Vec<Effect>,
    ) -> ExecutionStepResult {
        ExecutionUnit::run_until_yield(self, budget, ctx, effects)
    }

    #[inline]
    fn drain_retired_state_hashes(&mut self) -> Vec<(u64, u64)> {
        ExecutionUnit::drain_retired_state_hashes(self)
    }

    #[inline]
    fn drain_retired_state_full(&mut self) -> Vec<(u64, [u64; 32], u64, u64, u64, u32)> {
        ExecutionUnit::drain_retired_state_full(self)
    }

    #[inline]
    fn drain_profile_insns(&mut self) -> Vec<(&'static str, u64)> {
        ExecutionUnit::drain_profile_insns(self)
    }

    #[inline]
    fn drain_profile_pairs(&mut self) -> Vec<((&'static str, &'static str), u64)> {
        ExecutionUnit::drain_profile_pairs(self)
    }

    #[inline]
    fn invalidate_code(&mut self, addr: u64, len: u64) {
        ExecutionUnit::invalidate_code(self, addr, len)
    }

    #[inline]
    fn shadow_stats(&self) -> (u64, u64) {
        ExecutionUnit::shadow_stats(self)
    }

    #[inline]
    fn register_dump(&self) -> Option<FaultRegisterDump> {
        ExecutionUnit::register_dump(self)
    }

    #[inline]
    fn as_any(&self) -> &dyn Any {
        self
    }
}
