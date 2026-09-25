//! The trace record type, the format version, and each record's tag, length and level.

use crate::hash::StateHash;
use crate::level::TraceLevel;
use cellgov_event::UnitId;
use cellgov_time::{Budget, Epoch, GuestTicks, InstructionCost};

use super::codec::{
    TAG_COMMIT_APPLIED, TAG_EFFECT_EMITTED, TAG_HOST_INVARIANT_BREAK, TAG_HOST_WRITE,
    TAG_PPU_STATE_FULL, TAG_PPU_STATE_HASH, TAG_RESERVED_REGION_READ, TAG_RUN_IDENTITY,
    TAG_STATE_HASH_CHECKPOINT, TAG_STATE_HASH_SCHEME, TAG_STEP_COMPLETED, TAG_SYSCALL_ENTERED,
    TAG_SYSCALL_RETURNED, TAG_UNIT_BLOCKED, TAG_UNIT_SCHEDULED, TAG_UNIT_WOKEN,
};
use super::reasons::{
    HashCheckpointKind, HostWriter, TracedBlockReason, TracedEffectKind,
    TracedInvariantBreakReason, TracedSyscallDisposition, TracedWakeReason, TracedYieldReason,
};

/// Version of the binary trace format, carried by the
/// [`TraceRecord::RunIdentity`] header record.
///
/// A stream whose first record is not `RunIdentity` is version 1 and
/// carries no run identity.
pub const TRACE_FORMAT_VERSION: u32 = 3;

/// A single structured trace record.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TraceRecord {
    /// Header record: the format version, then fingerprints of the run's
    /// identity triple and boot overrides.
    ///
    /// The header leads the stream and never repeats.
    RunIdentity {
        /// [`TRACE_FORMAT_VERSION`] at the time the stream was written.
        format_version: u32,
        /// Fingerprint of the firmware half; 0 when the run named no
        /// managed firmware.
        firmware: u64,
        /// Fingerprint of the game half; 0 when the run named no store
        /// entry.
        game: u64,
        /// Fingerprint of the boot overrides the run applied; 0 when it
        /// applied none.
        overrides: u64,
    },
    /// Scheduler selected a unit and granted it a budget.
    UnitScheduled {
        /// Unit that was scheduled.
        unit: UnitId,
        /// Budget granted to the unit for this step.
        granted_budget: Budget,
        /// Guest-time clock at scheduling.
        time: GuestTicks,
        /// Commit epoch at scheduling.
        epoch: Epoch,
    },
    /// A unit's `run_until_yield` returned.
    StepCompleted {
        /// Unit that completed the step.
        unit: UnitId,
        /// Reason the unit yielded.
        yield_reason: TracedYieldReason,
        /// Instruction cost consumed during the step.
        consumed_cost: InstructionCost,
        /// Guest-time clock after the step.
        time_after: GuestTicks,
    },
    /// Commit pipeline finished processing a step's effects.
    CommitApplied {
        /// Unit whose step was committed.
        unit: UnitId,
        /// Number of writes that landed in shared memory.
        writes_committed: u32,
        /// Number of effects deferred past this commit.
        effects_deferred: u32,
        /// Whether the step's effects were discarded due to a fault.
        fault_discarded: bool,
        /// Commit epoch after this commit.
        epoch_after: Epoch,
    },
    /// State hash captured at a controlled checkpoint.
    StateHashCheckpoint {
        /// Which subsystem the hash covers.
        kind: HashCheckpointKind,
        /// Hash value.
        hash: StateHash,
    },
    /// One record per effect, in emission order, with `sequence` running 0..N
    /// within the step. Effect payloads (write bytes, mailbox messages, DMA
    /// descriptors) are not included.
    EffectEmitted {
        /// Unit that emitted the effect.
        unit: UnitId,
        /// Per-step emission sequence number.
        sequence: u32,
        /// Effect variant.
        kind: TracedEffectKind,
    },
    /// Status overridden to `Blocked`. Emitted once per transition, after
    /// `CommitApplied`.
    UnitBlocked {
        /// Unit that transitioned to blocked.
        unit: UnitId,
        /// Why the unit blocked.
        reason: TracedBlockReason,
    },
    /// Status overridden to `Runnable` by the commit pipeline, a DMA
    /// completion, or a timer deadline. Emitted once per transition,
    /// after `CommitApplied`.
    UnitWoken {
        /// Unit that transitioned to runnable.
        unit: UnitId,
        /// Why the unit was woken.
        reason: TracedWakeReason,
    },
    /// Per-step PPU state fingerprint at instruction retire.
    ///
    /// `hash` covers GPR + LR + CTR + XER + CR and the reservation, under
    /// the scheme the stream's [`StateHashScheme`](Self::StateHashScheme)
    /// record names. The stream carries one per retired instruction
    /// while per-step tracing is active.
    ///
    /// FPR, VMX, and FPSCR are outside the covered set. A divergence confined
    /// to float or vector state stays invisible until it reaches a covered
    /// register, so the step a consumer derives from these records is the
    /// first scalar-visible divergence, not necessarily the first divergence.
    PpuStateHash {
        /// Per-thread retired-instruction counter.
        step: u64,
        /// PC of the instruction that just retired.
        pc: u64,
        /// Hash of the PPU architectural state.
        hash: StateHash,
    },
    /// Scalar PPU register snapshot at instruction retire.
    ///
    /// Opt-in `[lo, hi]` window only, never on the hot path. Carries the
    /// full fingerprint input set of [`PpuStateHash`](Self::PpuStateHash)
    /// uncompressed -- GPR, LR, CTR, XER, CR, reservation; no float or
    /// vector state -- so a hash divergence always names the disagreeing
    /// field in the zoom diff. `step` matches `PpuStateHash::step` for
    /// the same instruction.
    PpuStateFull {
        /// Per-thread retired-instruction counter.
        step: u64,
        /// PC of the instruction that just retired.
        pc: u64,
        /// General-purpose registers r0..r31.
        gpr: [u64; 32],
        /// Link register.
        lr: u64,
        /// Count register.
        ctr: u64,
        /// Fixed-point exception register.
        xer: u64,
        /// Condition register.
        cr: u32,
        /// Active reservation's 128-byte line address, if held.
        reservation_line: Option<u64>,
    },
    /// Host-side invariant break observed during `Lv2Host::dispatch`.
    /// One record per `record_invariant_break` call in `cellgov_lv2`.
    HostInvariantBreak {
        /// Category of the break.
        reason: TracedInvariantBreakReason,
    },
    /// One record per syscall entry, emitted by the runtime before
    /// `Lv2Host::dispatch` runs (or, for the `TIMER_USLEEP` /
    /// `TIMER_SLEEP` fast-path, before guest-time advance).
    SyscallEntered {
        /// Requester unit id (the PPU that issued `sc`).
        unit: UnitId,
        /// Raw value the dispatcher saw at `result.syscall_args[0]`:
        /// LV2 syscall number, hypercall number, or the synthetic
        /// `UNRESOLVED_IMPORT` sentinel for HLE trampolines.
        num: u64,
        /// Argument registers GPR 3..=10 in declaration order.
        args: [u64; 8],
        /// Classified dispatch arm.
        disposition: TracedSyscallDisposition,
    },
    /// Reads of a reserved-zero-readable region (RSX / SPU ranges the
    /// runtime models as provisional zeros), one record per distinct
    /// `(addr, len)` drained after a step or commit. The zeros the
    /// guest saw were a modeling choice, so a replay comparison can
    /// find where that choice touched the run.
    ReservedRegionRead {
        /// Unit whose step (or commit) the reads are attributed to.
        unit: UnitId,
        /// Runtime step count at the drain.
        step: u64,
        /// Guest address of the first byte read.
        addr: u64,
        /// Bytes read.
        len: u32,
        /// Reads of exactly this `(addr, len)` since the previous drain.
        hits: u32,
    },
    /// The value a syscall handed back to its caller, emitted when the
    /// runtime stores it for the caller's next step: in the dispatching
    /// commit for an immediate return, at wake time for a call that
    /// blocked. Pairs with the caller's most recent
    /// [`SyscallEntered`](Self::SyscallEntered).
    SyscallReturned {
        /// Caller the value is delivered to.
        unit: UnitId,
        /// Value written to the caller's `r3`.
        code: u64,
        /// Guest-time clock when the value was delivered.
        time: GuestTicks,
    },
    /// A write the runtime itself landed in guest memory, one record
    /// per accepted write. A refused write emits nothing: it changed
    /// no guest-visible byte.
    HostWrite {
        /// Host mechanism that produced the write.
        writer: HostWriter,
        /// Address space the write landed in; 0 is the boot space.
        space: u32,
        /// Guest address of the first byte written.
        addr: u64,
        /// Bytes written.
        len: u32,
        /// Reservations the write's clear sweep dropped.
        reservations_cleared: u32,
    },
    /// The scheme ids of the stream's state hashes.
    ///
    /// A run writes it once, directly after the
    /// [`RunIdentity`](Self::RunIdentity) header. A stream without it
    /// holds hashes of the FNV-1a PPU scheme and of the first checkpoint
    /// scheme.
    StateHashScheme {
        /// Scheme id of every [`PpuStateHash`](Self::PpuStateHash) in the
        /// stream.
        ppu: u64,
        /// Scheme id of every
        /// [`StateHashCheckpoint`](Self::StateHashCheckpoint) in the
        /// stream.
        checkpoint: u64,
    },
}

impl TraceRecord {
    /// Tag byte that leads this record on the wire.
    pub fn tag(&self) -> u8 {
        match self {
            TraceRecord::RunIdentity { .. } => TAG_RUN_IDENTITY,
            TraceRecord::UnitScheduled { .. } => TAG_UNIT_SCHEDULED,
            TraceRecord::StepCompleted { .. } => TAG_STEP_COMPLETED,
            TraceRecord::CommitApplied { .. } => TAG_COMMIT_APPLIED,
            TraceRecord::StateHashCheckpoint { .. } => TAG_STATE_HASH_CHECKPOINT,
            TraceRecord::EffectEmitted { .. } => TAG_EFFECT_EMITTED,
            TraceRecord::UnitBlocked { .. } => TAG_UNIT_BLOCKED,
            TraceRecord::UnitWoken { .. } => TAG_UNIT_WOKEN,
            TraceRecord::PpuStateHash { .. } => TAG_PPU_STATE_HASH,
            TraceRecord::PpuStateFull { .. } => TAG_PPU_STATE_FULL,
            TraceRecord::HostInvariantBreak { .. } => TAG_HOST_INVARIANT_BREAK,
            TraceRecord::SyscallEntered { .. } => TAG_SYSCALL_ENTERED,
            TraceRecord::ReservedRegionRead { .. } => TAG_RESERVED_REGION_READ,
            TraceRecord::SyscallReturned { .. } => TAG_SYSCALL_RETURNED,
            TraceRecord::HostWrite { .. } => TAG_HOST_WRITE,
            TraceRecord::StateHashScheme { .. } => TAG_STATE_HASH_SCHEME,
        }
    }

    /// Encoded length, tag byte included, of a record led by `tag`;
    /// `None` for a tag no variant owns.
    pub const fn encoded_len(tag: u8) -> Option<usize> {
        Some(match tag {
            TAG_UNIT_SCHEDULED => 1 + 8 * 4,
            TAG_STEP_COMPLETED => 1 + 8 + 1 + 8 + 8,
            TAG_COMMIT_APPLIED => 1 + 8 + 4 + 4 + 1 + 8,
            TAG_STATE_HASH_CHECKPOINT => 1 + 1 + 8,
            TAG_EFFECT_EMITTED => 1 + 8 + 4 + 1,
            TAG_UNIT_BLOCKED => 1 + 8 + 1,
            TAG_UNIT_WOKEN => 1 + 8 + 1,
            TAG_PPU_STATE_HASH => 1 + 8 * 3,
            TAG_PPU_STATE_FULL => 1 + 8 + 8 + 8 * 32 + 8 * 3 + 4 + 1 + 8,
            TAG_HOST_INVARIANT_BREAK => 1 + 1,
            TAG_SYSCALL_ENTERED => 1 + 8 + 8 + 8 * 8 + 1,
            TAG_RESERVED_REGION_READ => 1 + 8 * 3 + 4 + 4,
            TAG_SYSCALL_RETURNED => 1 + 8 * 3,
            TAG_RUN_IDENTITY => 1 + 4 + 8 * 3,
            TAG_HOST_WRITE => 1 + 1 + 4 + 8 + 4 + 4,
            TAG_STATE_HASH_SCHEME => 1 + 8 + 8,
            _ => return None,
        })
    }

    /// Trace level this record belongs to.
    pub fn level(&self) -> TraceLevel {
        match self {
            TraceRecord::RunIdentity { .. } => TraceLevel::Scheduling,
            TraceRecord::UnitScheduled { .. }
            | TraceRecord::StepCompleted { .. }
            | TraceRecord::UnitBlocked { .. }
            | TraceRecord::UnitWoken { .. } => TraceLevel::Scheduling,
            TraceRecord::CommitApplied { .. } => TraceLevel::Commits,
            TraceRecord::StateHashCheckpoint { .. }
            | TraceRecord::PpuStateHash { .. }
            | TraceRecord::PpuStateFull { .. } => TraceLevel::Hashes,
            TraceRecord::EffectEmitted { .. } => TraceLevel::Effects,
            TraceRecord::HostInvariantBreak { .. } => TraceLevel::Scheduling,
            TraceRecord::SyscallEntered { .. } => TraceLevel::Scheduling,
            TraceRecord::ReservedRegionRead { .. } => TraceLevel::Hashes,
            TraceRecord::SyscallReturned { .. } => TraceLevel::Scheduling,
            TraceRecord::HostWrite { .. } => TraceLevel::Commits,
            TraceRecord::StateHashScheme { .. } => TraceLevel::Hashes,
        }
    }
}
