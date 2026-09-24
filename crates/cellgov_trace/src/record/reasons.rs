//! The reason and kind enums each record carries, mirrored by raw value from their source enums.

use num_enum::{IntoPrimitive, TryFromPrimitive};

use super::error::DecodeError;

/// Mirror of `cellgov_exec::YieldReason` for the trace stream.
///
/// Discriminants must match the source enum: the trace crate cannot depend on
/// `cellgov_exec` (DAG: effects -> exec, effects -> trace), so the bridge maps
/// by raw value.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, IntoPrimitive, TryFromPrimitive, strum::VariantArray,
)]
#[repr(u8)]
#[num_enum(error_type(name = DecodeError, constructor = DecodeError::unknown_yield_reason))]
pub enum TracedYieldReason {
    /// Unit consumed its full granted budget.
    BudgetExhausted = 0,
    /// Unit yielded for mailbox access.
    MailboxAccess = 1,
    /// Unit submitted a DMA and yielded.
    DmaSubmitted = 2,
    /// Unit yielded waiting for DMA completion.
    DmaWait = 3,
    /// Unit yielded waiting on a sync primitive.
    WaitingSync = 4,
    /// Unit yielded on a syscall.
    Syscall = 5,
    /// Unit yielded at an interrupt boundary.
    InterruptBoundary = 6,
    /// Unit yielded due to a fault.
    Fault = 7,
    /// Unit reached its terminal state.
    Finished = 8,
    /// `sc` with LEV >= 1 (hypercall). PS3 usermode never issues these;
    /// distinguished from `Syscall` so a rejection cannot byte-collide with an
    /// unrelated LV2 handler returning `CELL_EINVAL`.
    Hypercall = 9,
}

/// Which piece of state a [`TraceRecord::StateHashCheckpoint`](super::TraceRecord::StateHashCheckpoint) hashes.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, IntoPrimitive, TryFromPrimitive, strum::VariantArray,
)]
#[repr(u8)]
#[num_enum(error_type(name = DecodeError, constructor = DecodeError::unknown_hash_kind))]
pub enum HashCheckpointKind {
    /// Hash covers committed shared memory.
    CommittedMemory = 0,
    /// Hash covers the runnable-unit queue.
    RunnableQueue = 1,
    /// Hash covers sync-primitive state.
    SyncState = 2,
    /// Hash covers per-unit status flags.
    UnitStatus = 3,
}

/// Why a unit was blocked, as the trace records it.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, IntoPrimitive, TryFromPrimitive, strum::VariantArray,
)]
#[repr(u8)]
#[num_enum(error_type(name = DecodeError, constructor = DecodeError::unknown_block_reason))]
pub enum TracedBlockReason {
    /// Unit blocked waiting on a sync event.
    WaitOnEvent = 0,
    /// Unit blocked because its mailbox was empty.
    MailboxEmpty = 1,
    /// SPU blocked on `MFC_RD_TAG_STAT` until a pending DMA completes.
    DmaWait = 2,
}

/// Why a unit was woken, as the trace records it.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, IntoPrimitive, TryFromPrimitive, strum::VariantArray,
)]
#[repr(u8)]
#[num_enum(error_type(name = DecodeError, constructor = DecodeError::unknown_wake_reason))]
pub enum TracedWakeReason {
    /// Unit woken by a wake effect.
    WakeEffect = 0,
    /// Unit woken by DMA completion.
    DmaCompletion = 1,
    /// Unit woken because guest time reached its timer deadline.
    Timer = 2,
}

/// Mirror of `cellgov_effects::Effect` for the trace stream.
///
/// A recorded trace holds these as raw bytes, so a discriminant that
/// moves changes what an existing stream decodes to. A variant
/// `cellgov_effects::Effect` gains belongs at the end of this list.
/// `cellgov_core::runtime::trace_bridge` pairs the two enums by name,
/// so the order answers to the wire format alone.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, IntoPrimitive, TryFromPrimitive, strum::VariantArray,
)]
#[repr(u8)]
#[num_enum(error_type(name = DecodeError, constructor = DecodeError::unknown_effect_kind))]
pub enum TracedEffectKind {
    /// Shared-memory write intent.
    SharedWriteIntent = 0,
    /// Mailbox send.
    MailboxSend = 1,
    /// Mailbox receive attempt.
    MailboxReceiveAttempt = 2,
    /// DMA descriptor enqueued.
    DmaEnqueue = 3,
    /// Wait on a sync event.
    WaitOnEvent = 4,
    /// Wake another unit.
    WakeUnit = 5,
    /// Sync signal update.
    SignalUpdate = 6,
    /// Fault raised by the unit.
    FaultRaised = 7,
    /// User-emitted trace marker.
    TraceMarker = 8,
    /// `lwarx` / `ldarx` / `getllar`.
    ReservationAcquire = 9,
    /// `stwcx` / `stdcx` / `putllc` success.
    ConditionalStore = 10,
    /// NV406E semaphore release or report writeback.
    RsxLabelWrite = 11,
    /// NV4097 flip.
    RsxFlipRequest = 12,
    /// Shared-memory read intent.
    SharedReadIntent = 13,
    /// Guest-clock read.
    ClockRead = 14,
}

/// Reason a host-side invariant break was recorded into the trace
/// stream. The bridge in `cellgov_core::runtime::trace_bridge` maps
/// the lv2-owned source enum onto this mirror by exhaustive match.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, IntoPrimitive, TryFromPrimitive, strum::VariantArray,
)]
#[repr(u8)]
#[num_enum(error_type(name = DecodeError, constructor = DecodeError::unknown_invariant_break_reason))]
pub enum TracedInvariantBreakReason {
    /// Catch-all placeholder emitted for every host invariant break
    /// observed during dispatch.
    Unspecified = 0,
}

/// Which host mechanism produced a [`TraceRecord::HostWrite`](super::TraceRecord::HostWrite).
///
/// A host write is a write to guest memory by the runtime itself,
/// outside any execution unit's committed effects. The stream has no
/// `UnitId` for such a write, so this enum names the mechanism in
/// that slot.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, IntoPrimitive, TryFromPrimitive, strum::VariantArray,
)]
#[repr(u8)]
#[num_enum(error_type(name = DecodeError, constructor = DecodeError::unknown_host_writer))]
pub enum HostWriter {
    /// An LV2 handler's `SharedWriteIntent`, which lands at dispatch
    /// time instead of through the commit pipeline.
    Lv2Effect = 0,
    /// A continuation payload written through a pointer a parked
    /// caller supplied, at the point its wait resolves.
    /// [`HostWriter::SyscallOutParam`] covers the write-back inside
    /// the dispatch itself.
    WakeContinuation = 1,
    /// A DMA transfer's payload, which lands when its modeled latency
    /// window arrives.
    DmaCompletion = 2,
    /// An RSX control-register or flip-status mirror slot.
    RsxMirror = 3,
    /// Replication of a committed write into a sibling view of the
    /// same shared mapping.
    SharedViewFanout = 4,
    /// The segment copy the runtime seeds a newly attached view with.
    SharedViewSeed = 5,
    /// A syscall out-parameter the runtime writes back at dispatch
    /// time, for a caller that never parked.
    SyscallOutParam = 6,
    /// Bytes the program driving the runtime places itself, outside
    /// every modeled mechanism.
    Placement = 7,
}

/// Which dispatch arm a [`TraceRecord::SyscallEntered`](super::TraceRecord::SyscallEntered) record was
/// classified into. Pure function of `(lev, num, args)`; derived by
/// the runtime before `Lv2Host::dispatch` runs.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, IntoPrimitive, TryFromPrimitive, strum::VariantArray,
)]
#[repr(u8)]
#[num_enum(error_type(name = DecodeError, constructor = DecodeError::unknown_syscall_disposition))]
pub enum TracedSyscallDisposition {
    /// Typed `Lv2Request::*` variant other than the catch-alls; a
    /// modeled syscall.
    Implemented = 0,
    /// `Lv2Request::Unsupported`: classifier recognized the number but
    /// no handler is wired; routes to `dispatch_unsupported_default`.
    Unsupported = 1,
    /// `Lv2Request::UnresolvedImport`: HLE trampoline fired for a NID
    /// the workspace has not bound.
    UnresolvedImport = 2,
    /// `Lv2Request::Malformed`: classifier rejected the request shape
    /// (bad argument-register read, narrowing, etc.).
    Malformed = 3,
    /// `Lv2Request::Hypercall`: LEV >= 1; PS3 usermode never issues
    /// these.
    Hypercall = 4,
    /// `TIMER_USLEEP` or `TIMER_SLEEP` short-circuit: bypasses
    /// `Lv2Host::dispatch` entirely; the caller parks on the runtime's
    /// timer-wake queue until guest time reaches its deadline.
    TimerFastPath = 5,
    /// `Lv2Request::NoSuchSyscall`: `r11` is outside the LV2 syscall table.
    NoSuchSyscall = 6,
}
