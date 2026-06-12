//! The `Effect` enum: the vocabulary an execution unit uses to request
//! runtime-visible work.

use crate::payload::{FaultKind, MailboxMessage, WaitTarget, WritePayload};
use cellgov_dma::DmaRequest;
use cellgov_event::{PriorityClass, UnitId};
use cellgov_mem::ByteRange;
use cellgov_sync::{MailboxId, SignalId};
use cellgov_time::GuestTicks;

/// An immutable effect packet emitted by an execution unit.
///
/// Every effect-consuming crate (commit pipeline, trace, LV2) codes
/// against this variant set and its field layout. Emission order is
/// preserved end-to-end; validation, conflict resolution, fault
/// attribution, and trace reconstruction all depend on stable
/// intra-step ordering even though commit batches are atomic from the
/// guest's standpoint. Variant discriminants are part of the binary
/// trace contract: new variants must be appended; existing variants
/// must not be reordered or have fields removed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    /// Stage a write to globally visible memory, applied at the next
    /// epoch boundary with the rest of its batch.
    SharedWriteIntent {
        /// Target byte range in the guest address space.
        range: ByteRange,
        /// Bytes to deposit; length checked against `range.length()`
        /// at commit validation.
        bytes: WritePayload,
        /// Tier-2 class for conflict resolution on overlapping writes.
        ordering: PriorityClass,
        /// Emitting unit.
        source: UnitId,
        /// Guest-time stamp at which this write becomes ordered.
        source_time: GuestTicks,
    },
    /// Send a message into a mailbox.
    MailboxSend {
        /// Destination mailbox.
        mailbox: MailboxId,
        /// Message word to deposit.
        message: MailboxMessage,
        /// Sending unit.
        source: UnitId,
    },
    /// Record the unit's intent to receive from a mailbox; the runtime
    /// decides block-vs-wake.
    MailboxReceiveAttempt {
        /// Mailbox to read from.
        mailbox: MailboxId,
        /// Receiving unit.
        source: UnitId,
    },
    /// Enqueue a DMA request for the runtime to model and complete.
    DmaEnqueue {
        /// The DMA request packet.
        request: DmaRequest,
        /// Inline bytes from unit-private memory (e.g. SPU local
        /// store); when present the commit pipeline writes these at
        /// completion instead of reading the source range.
        payload: Option<Vec<u8>>,
    },
    /// Block this unit until the named event fires.
    WaitOnEvent {
        /// What the unit is waiting for.
        target: WaitTarget,
        /// Blocking unit.
        source: UnitId,
    },
    /// Wake another unit directly, outside of any sync primitive.
    WakeUnit {
        /// Unit to wake.
        target: UnitId,
        /// Unit performing the wake.
        source: UnitId,
    },
    /// OR `value` into a signal notification register at commit time.
    SignalUpdate {
        /// Target signal register.
        signal: SignalId,
        /// Bits to OR into the register.
        value: u32,
        /// Emitting unit.
        source: UnitId,
    },
    /// Raise a fault; every effect emitted in this step is discarded
    /// (including those preceding `FaultRaised` in emission order).
    FaultRaised {
        /// Fault classification.
        kind: FaultKind,
        /// Faulting unit.
        source: UnitId,
    },
    /// Drop a debug breadcrumb into the trace. No commit-pipeline
    /// side-effect.
    TraceMarker {
        /// Opaque marker value, recorded as-is.
        marker: u32,
        /// Emitting unit.
        source: UnitId,
    },
    /// Install an atomic-reservation entry for `source` on the cache
    /// line containing `line_addr`.
    ///
    /// The pipeline canonicalizes `line_addr` to a 128-byte-aligned
    /// line (low 7 bits masked); callers may pass any byte-granular
    /// address within the intended line. A second acquire from the
    /// same unit replaces any prior entry; a later committed write
    /// overlapping the reserved line clears it.
    ReservationAcquire {
        /// Byte address within the line to reserve.
        line_addr: u64,
        /// Unit installing the reservation.
        source: UnitId,
    },
    /// Success path of a conditional atomic store (`stwcx.` / `stdcx.`
    /// / `MFC_PUTLLC`).
    ///
    /// Commits `bytes` into `range` through the `SharedWriteIntent`
    /// path (including the clear sweep over other units' overlapping
    /// reservations), then retires `source`'s own reservation entry.
    /// `range.length()` must be 4, 8, or 128. Presence encodes
    /// success; on failure the unit emits nothing and sets its own
    /// CR0 EQ / atomic_status bit locally, deciding before emission
    /// via `ExecutionContext::reservation_held(source)`.
    ConditionalStore {
        /// Target byte range; width must be 4, 8, or 128.
        range: ByteRange,
        /// Bytes to deposit; length must equal `range.length()`.
        bytes: WritePayload,
        /// Tier-2 class for conflict resolution on overlapping writes.
        ordering: PriorityClass,
        /// Unit whose reservation entry this commit retires.
        source: UnitId,
        /// Guest-time stamp at which the store becomes ordered.
        source_time: GuestTicks,
    },
    /// An NV method wrote a 32-bit value into the RSX label area
    /// (NV406E semaphore release or NV4097 report writeback).
    ///
    /// The pipeline resolves `offset` against the RSX label base and
    /// commits big-endian through the `SharedWriteIntent` path, so
    /// clear sweep and state-hash contribution are automatic.
    /// Distinct variant so the trace records the FIFO origin. No
    /// `source: UnitId` because the FIFO advance pass runs inside
    /// the commit pipeline, not as a unit step.
    RsxLabelWrite {
        /// Byte offset into the RSX label area.
        offset: u32,
        /// 32-bit value; emitted big-endian at commit time.
        value: u32,
    },
    /// An `NV4097_FLIP_BUFFER` method parsed by the FIFO advance
    /// pass; drives the flip-status state machine (WAITING, then
    /// DONE at the next commit boundary) with no memory side-effect.
    ///
    /// No `source: UnitId` for the same reason as
    /// [`Effect::RsxLabelWrite`].
    RsxFlipRequest {
        /// Back-buffer index recorded for observability; the state
        /// machine only tracks pending vs done.
        buffer_index: u8,
    },
}

#[cfg(test)]
#[path = "tests/effect_tests.rs"]
mod tests;
