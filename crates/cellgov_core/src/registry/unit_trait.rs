//! Object-safe view of [`ExecutionUnit`] for the runtime registry.
//! [`ExecutionUnit`] has an associated `Snapshot` so it is not
//! object-safe; [`RegisteredUnit`] mirrors the runtime-visible methods
//! and is blanket-impl'd for every `U: ExecutionUnit + Clone + 'static`.

use core::any::Any;

use cellgov_effects::Effect;
use cellgov_event::UnitId;
use cellgov_exec::{
    ExecutionContext, ExecutionStepResult, ExecutionUnit, FaultRegisterDump, PpuFingerprint,
    ProblemStateError, RestartError, SignalNotifier, StopRegisters, UnitStatus,
};
use cellgov_time::Budget;

/// Object-safe view of an execution unit.
///
/// Method contracts mirror [`ExecutionUnit`]; see that trait for
/// the authoritative docs.
pub trait RegisteredUnit: 'static {
    /// Stable id assigned at registration.
    fn unit_id(&self) -> UnitId;

    /// Deep-clone guest state behind a fresh box.
    ///
    /// Observers held in shared pointers may remain shared across a
    /// snapshot fork. They report diagnostics and do not affect guest
    /// execution.
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
    fn drain_retired_state_full(&mut self) -> Vec<(u64, u64, PpuFingerprint)>;

    /// Drain instruction-variant profiling data.
    /// See [`ExecutionUnit::drain_profile_insns`].
    fn drain_profile_insns(&mut self) -> Vec<(&'static str, u64)>;

    /// Drain adjacent-pair profiling data.
    /// See [`ExecutionUnit::drain_profile_pairs`].
    fn drain_profile_pairs(&mut self) -> Vec<((&'static str, &'static str), u64)>;

    /// Notify the unit that guest memory in `[addr, addr+len)` was written.
    /// See [`ExecutionUnit::invalidate_code`] for the must-override contract.
    ///
    /// The call must not change what [`Self::status`] reports: the
    /// registry calls it without marking the unit's status lane stale.
    fn invalidate_code(&mut self, addr: u64, len: u64);

    /// Whether this unit caches decoded guest code.
    fn caches_code(&self) -> bool;

    /// Shadow hit/miss counters. See [`ExecutionUnit::shadow_stats`].
    fn shadow_stats(&self) -> (u64, u64);

    /// Hash of the unit's private memory, or `None`. See
    /// [`ExecutionUnit::local_memory_hash`].
    fn local_memory_hash(&self) -> Option<u64>;

    /// Current register snapshot for diagnostic dumps. See
    /// [`ExecutionUnit::register_dump`].
    fn register_dump(&self) -> Option<FaultRegisterDump>;

    /// The unit's self-stopped state. See
    /// [`ExecutionUnit::stop_registers`].
    fn stop_registers(&self) -> Option<StopRegisters>;

    /// Resume a self-stopped unit. See [`ExecutionUnit::restart`].
    ///
    /// # Errors
    ///
    /// [`RestartError::NotStopped`] when the unit holds no stopped state.
    fn restart(&mut self) -> Result<(), RestartError>;

    /// The `SPU_Status` word. See [`ExecutionUnit::spu_status`].
    fn spu_status(&self) -> Option<u32>;

    /// The channel access the unit stalled on. See
    /// [`ExecutionUnit::channel_stall`].
    fn channel_stall(&self) -> Option<cellgov_exec::ChannelStall>;

    /// An `SPU_RunCntl` stop request. See [`ExecutionUnit::request_stop`].
    ///
    /// # Errors
    ///
    /// [`ProblemStateError::NoProblemState`] for a unit without SPE
    /// problem-state registers.
    fn request_stop(&mut self, waiting: bool) -> Result<(), ProblemStateError>;

    /// Write `SPU_NPC`. See [`ExecutionUnit::write_npc`].
    ///
    /// # Errors
    ///
    /// [`ProblemStateError::NoProblemState`] or
    /// [`ProblemStateError::Running`].
    fn write_npc(&mut self, npc: u32) -> Result<(), ProblemStateError>;

    /// Write a signal-notification register. See
    /// [`ExecutionUnit::write_signal`].
    ///
    /// # Errors
    ///
    /// [`ProblemStateError::NoProblemState`] for a unit without SPE
    /// problem-state registers.
    fn write_signal(
        &mut self,
        register: SignalNotifier,
        value: u32,
    ) -> Result<(), ProblemStateError>;

    /// Read `SPU_Out_Mbox`. See [`ExecutionUnit::read_out_mbox`].
    ///
    /// # Errors
    ///
    /// [`ProblemStateError::NoProblemState`] for a unit without SPE
    /// problem-state registers.
    fn read_out_mbox(&mut self) -> Result<Option<u32>, ProblemStateError>;

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
    fn drain_retired_state_full(&mut self) -> Vec<(u64, u64, PpuFingerprint)> {
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
    fn caches_code(&self) -> bool {
        ExecutionUnit::caches_code(self)
    }

    #[inline]
    fn shadow_stats(&self) -> (u64, u64) {
        ExecutionUnit::shadow_stats(self)
    }

    #[inline]
    fn local_memory_hash(&self) -> Option<u64> {
        ExecutionUnit::local_memory_hash(self)
    }

    #[inline]
    fn register_dump(&self) -> Option<FaultRegisterDump> {
        ExecutionUnit::register_dump(self)
    }

    #[inline]
    fn stop_registers(&self) -> Option<StopRegisters> {
        ExecutionUnit::stop_registers(self)
    }

    #[inline]
    fn restart(&mut self) -> Result<(), RestartError> {
        ExecutionUnit::restart(self)
    }

    #[inline]
    fn spu_status(&self) -> Option<u32> {
        ExecutionUnit::spu_status(self)
    }

    #[inline]
    fn channel_stall(&self) -> Option<cellgov_exec::ChannelStall> {
        ExecutionUnit::channel_stall(self)
    }

    #[inline]
    fn request_stop(&mut self, waiting: bool) -> Result<(), ProblemStateError> {
        ExecutionUnit::request_stop(self, waiting)
    }

    #[inline]
    fn write_npc(&mut self, npc: u32) -> Result<(), ProblemStateError> {
        ExecutionUnit::write_npc(self, npc)
    }

    #[inline]
    fn write_signal(
        &mut self,
        register: SignalNotifier,
        value: u32,
    ) -> Result<(), ProblemStateError> {
        ExecutionUnit::write_signal(self, register, value)
    }

    #[inline]
    fn read_out_mbox(&mut self) -> Result<Option<u32>, ProblemStateError> {
        ExecutionUnit::read_out_mbox(self)
    }

    #[inline]
    fn as_any(&self) -> &dyn Any {
        self
    }
}
