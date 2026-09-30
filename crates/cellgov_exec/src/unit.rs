//! The `ExecutionUnit` trait and `UnitStatus` enum.

use crate::context::ExecutionContext;
use crate::step_result::ExecutionStepResult;
use cellgov_effects::Effect;
use cellgov_event::UnitId;
use cellgov_time::Budget;

/// Coarse runnability state queried by the scheduler.
///
/// Finer-grained reasons for the most recent yield live in
/// [`crate::YieldReason`]; internal arch state lives on the unit
/// itself.
///
/// Discriminants are part of the binary trace format.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, strum::VariantArray)]
#[repr(u8)]
pub enum UnitStatus {
    /// Eligible to be scheduled.
    Runnable = 0,
    /// Parked, waiting on an external event. The scheduler sees only
    /// the opaque state and skips the unit. The subsystem that parks
    /// the unit keeps the reason:
    ///
    /// - the commit pipeline in `cellgov_core`, for mailbox, event and
    ///   DMA waits;
    /// - `cellgov_lv2`, for LV2 waits such as a PPU thread `join`.
    Blocked = 1,
    /// Has raised a fault; kept out of the runnable set. Return to
    /// `Runnable` is architecture-specific.
    Faulted = 2,
    /// Out of the runnable set after the runtime observes this; the
    /// runtime can keep its snapshots for the trace. Terminal
    /// unless the unit reports a stopped state through
    /// [`ExecutionUnit::stop_registers`], which [`ExecutionUnit::restart`]
    /// resumes from.
    Finished = 3,
}

/// Canonical PPU fingerprint input set.
///
/// One field list shared by three consumers: `PpuState::state_hash`
/// folds exactly these fields, `TraceRecord::PpuStateFull` carries
/// them, and the zoom diff walker enumerates them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PpuFingerprint {
    /// General-purpose registers r0..r31.
    pub gpr: [u64; 32],
    /// Link register.
    pub lr: u64,
    /// Count register.
    pub ctr: u64,
    /// Fixed-point exception register.
    pub xer: u64,
    /// Condition register.
    pub cr: u32,
    /// Active reservation's 128-byte line address, if held.
    pub reservation_line: Option<u64>,
}

/// Canonical SPU fingerprint input set.
///
/// The SPU counterpart of [`PpuFingerprint`]: `SpuState::state_hash`
/// folds exactly these fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpuFingerprint {
    /// Registers r0..r127, byte 0 of each the most significant.
    pub regs: [u128; 128],
    /// Floating-point status and control register.
    pub fpscr: u128,
    /// Local storage limit register.
    pub lslr: u32,
    /// Interrupt-enable state.
    pub interrupts_enabled: bool,
    /// State save and restore register 0.
    pub srr0: u32,
    /// Line address of the local reservation, if held.
    pub reservation_line: Option<u64>,
}

/// A resumable execution unit: something that can take a budget, run
/// for some guest time, and return a step result.
///
/// Implementations communicate with the runtime through
/// `ExecutionContext` input and `Effect` output only. They do not
/// import scheduler types and do not mutate guest-visible state
/// directly.
///
/// **Snapshot rule (required for replay).** `Self::Snapshot` must be
/// pure deterministic data: no raw pointers, no host handles, no
/// allocator-dependent internals, no mutex guards, no references
/// into runtime-owned memory. A snapshot must be reconstructible
/// into an equivalent unit state on a different host.
pub trait ExecutionUnit {
    /// Pure deterministic state capture used for replay and assertions.
    type Snapshot;

    /// Stable identifier assigned at registration time.
    fn unit_id(&self) -> UnitId;

    /// Coarse runnability state queried by the scheduler.
    fn status(&self) -> UnitStatus;

    /// Run until the unit yields, consuming up to `budget` and
    /// observing only the readonly state in `ctx`.
    ///
    /// Effects are pushed into `effects` in emission order. The
    /// runtime relies on stable intra-step ordering for validation,
    /// conflict diagnostics, fault attribution, and trace
    /// reconstruction.
    fn run_until_yield(
        &mut self,
        budget: Budget,
        ctx: &ExecutionContext<'_>,
        effects: &mut Vec<Effect>,
    ) -> ExecutionStepResult;

    /// Capture current state as deterministic data per the snapshot
    /// rule on the trait.
    fn snapshot(&self) -> Self::Snapshot;

    /// Drain `(pc, state_hash)` pairs retired during the most recent
    /// `run_until_yield`, in retirement order. The default returns an
    /// empty vec and allocates nothing.
    ///
    /// The caller assigns monotonic step indices; the unit does not
    /// know its own position in the global step sequence.
    fn drain_retired_state_hashes(&mut self) -> Vec<(u64, u64)> {
        Vec::new()
    }

    /// Drain full-register snapshots collected during the most recent
    /// `run_until_yield` inside the unit's configured zoom-in window.
    /// Each entry is `(step, pc, fingerprint)` in retirement order,
    /// where `step` is the unit's retirement counter at capture -- the
    /// same units the window bounds use -- so a window that opens
    /// mid-run still stamps each snapshot with its true step.
    fn drain_retired_state_full(&mut self) -> Vec<(u64, u64, PpuFingerprint)> {
        Vec::new()
    }

    /// Drain the barrier instructions retired during the most recent
    /// `run_until_yield`, in retirement order. The default returns an
    /// empty vec and allocates nothing.
    ///
    /// A unit collects them only when
    /// [`ExecutionContext::trace_per_step`] is true.
    fn drain_barriers(&mut self) -> Vec<crate::RetiredBarrier> {
        Vec::new()
    }

    /// Drain instruction-variant frequency data from profiling mode.
    fn drain_profile_insns(&mut self) -> Vec<(&'static str, u64)> {
        Vec::new()
    }

    /// Drain adjacent-pair frequency data from profiling mode.
    fn drain_profile_pairs(&mut self) -> Vec<((&'static str, &'static str), u64)> {
        Vec::new()
    }

    /// Notify the unit that guest memory in `[addr, addr+len)` was
    /// written by the commit pipeline.
    ///
    /// Any unit that caches decoded instructions, a translation-block
    /// index, a shadow PC ring, or anything else derived from guest
    /// code must override this to mark affected slots stale; the
    /// default no-op is correct only for units that derive nothing
    /// from guest code (synthetic / scenario units).
    fn invalidate_code(&mut self, _addr: u64, _len: u64) {}

    /// Whether this unit holds guest-code-derived state that needs invalidation.
    fn caches_code(&self) -> bool {
        false
    }

    /// Hash of the unit's private memory the guest reads back, or `None`
    /// for a unit with none.
    ///
    /// An SPU's local store is such memory: a program computes in it,
    /// and two schedules can leave the committed memory equal and the
    /// local store different. The schedule explorer folds this hash
    /// beside the committed-memory hash into the observable its verdict
    /// compares. Registers stay outside this hash.
    fn local_memory_hash(&self) -> Option<u64> {
        None
    }

    /// Return `(shadow_hits, shadow_misses)` for units with a
    /// predecoded instruction shadow; others report `(0, 0)`. A
    /// high miss ratio indicates fetches outside the shadowed
    /// region (e.g. PRX bodies) falling back to decode-on-fetch.
    fn shadow_stats(&self) -> (u64, u64) {
        (0, 0)
    }

    /// Snapshot the unit's current arch-neutral register state for
    /// diagnostic dumps (e.g. CLI `COMMIT_FAULT` formatters where the
    /// step did not produce a PC-side fault). Synthetic units leave
    /// this `None`; PPU / SPU implementations override to return the
    /// post-step register state.
    fn register_dump(&self) -> Option<crate::FaultRegisterDump> {
        None
    }

    /// The stopped state the unit's own instruction left it in, or
    /// `None`. An SPU reports it after a stop, a halt or an SPU error;
    /// other units never stop themselves this way.
    fn stop_registers(&self) -> Option<crate::StopRegisters> {
        None
    }

    /// Resume a unit that stopped itself, at the address its stopped
    /// state names. The unit becomes `Runnable` and its stopped state
    /// clears.
    ///
    /// # Errors
    ///
    /// [`crate::RestartError::NotStopped`] when the unit holds no
    /// stopped state.
    fn restart(&mut self) -> Result<(), crate::RestartError> {
        Err(crate::RestartError::NotStopped)
    }

    /// The `SPU_Status` word, or `None` for a unit without SPE
    /// problem-state registers.
    fn spu_status(&self) -> Option<u32> {
        None
    }

    /// The channel access the unit's last step stalled on, or `None`.
    /// The runtime reads it when the step yields
    /// [`crate::YieldReason::ChannelStall`], and a waker checks it so
    /// that only the channel's own producer wakes the unit.
    fn channel_stall(&self) -> Option<crate::ChannelStall> {
        None
    }

    /// The line address the unit's local reservation register holds, or
    /// `None` when it holds none. The runtime compares it with the
    /// committed reservation table to find a reservation another
    /// unit's store cleared.
    fn local_reservation(&self) -> Option<u64> {
        None
    }

    /// Write `bytes` into the unit's local store at `lsa`: the landing of
    /// an MFC get the unit queued, which completes between its steps.
    ///
    /// # Errors
    ///
    /// [`crate::ProblemStateError::NoProblemState`] for a unit without a
    /// local store; [`crate::ProblemStateError::Refused`] when the range
    /// leaves the local store.
    fn land_local_store(
        &mut self,
        _lsa: u32,
        _bytes: &[u8],
    ) -> Result<(), crate::ProblemStateError> {
        Err(crate::ProblemStateError::NoProblemState)
    }

    /// Read `len` bytes of the unit's local store from `lsa`: the source
    /// of another unit's MFC get through the unit's alias.
    ///
    /// # Errors
    ///
    /// [`crate::ProblemStateError::NoProblemState`] for a unit without a
    /// local store.
    fn read_local_store(&self, _lsa: u32, _len: u32) -> Result<Vec<u8>, crate::ProblemStateError> {
        Err(crate::ProblemStateError::NoProblemState)
    }

    /// An `SPU_RunCntl` stop request. `waiting` says the unit waits on a
    /// blocked channel. A stopped unit stays as it is.
    ///
    /// # Errors
    ///
    /// [`crate::ProblemStateError::NoProblemState`] for a unit without
    /// SPE problem-state registers.
    fn request_stop(&mut self, waiting: bool) -> Result<(), crate::ProblemStateError> {
        let _ = waiting;
        Err(crate::ProblemStateError::NoProblemState)
    }

    /// Write `SPU_NPC`: the address a restart resumes at.
    ///
    /// # Errors
    ///
    /// - [`crate::ProblemStateError::NoProblemState`] for a unit without
    ///   SPE problem-state registers.
    /// - [`crate::ProblemStateError::Running`] while the unit runs.
    fn write_npc(&mut self, npc: u32) -> Result<(), crate::ProblemStateError> {
        let _ = npc;
        Err(crate::ProblemStateError::NoProblemState)
    }

    /// Write one signal-notification register, in the mode its
    /// configuration selects.
    ///
    /// # Errors
    ///
    /// [`crate::ProblemStateError::NoProblemState`] for a unit without
    /// SPE problem-state registers.
    fn write_signal(
        &mut self,
        register: crate::SignalNotifier,
        value: u32,
    ) -> Result<(), crate::ProblemStateError> {
        let _ = (register, value);
        Err(crate::ProblemStateError::NoProblemState)
    }

    /// Set whether one signal-notification register ORs the data
    /// written into it (`true`) or overwrites its contents (`false`).
    ///
    /// # Errors
    ///
    /// [`crate::ProblemStateError::NoProblemState`] for a unit without
    /// SPE problem-state registers.
    fn set_signal_logical_or(
        &mut self,
        register: crate::SignalNotifier,
        logical_or: bool,
    ) -> Result<(), crate::ProblemStateError> {
        let _ = (register, logical_or);
        Err(crate::ProblemStateError::NoProblemState)
    }

    /// Read `SPU_Out_Mbox`: the oldest message the unit wrote, which
    /// leaves the mailbox, or `None` when it is empty.
    ///
    /// # Errors
    ///
    /// [`crate::ProblemStateError::NoProblemState`] for a unit without
    /// SPE problem-state registers.
    fn read_out_mbox(&mut self) -> Result<Option<u32>, crate::ProblemStateError> {
        Err(crate::ProblemStateError::NoProblemState)
    }
}

#[cfg(test)]
#[path = "tests/unit_tests.rs"]
mod tests;
