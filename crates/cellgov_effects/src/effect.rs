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
mod tests {
    use super::*;
    use cellgov_dma::{DmaDirection, DmaRequest};
    use cellgov_mem::{ByteRange, GuestAddr};
    use cellgov_sync::{BarrierId, MailboxId, SignalId};

    fn range(start: u64, length: u64) -> ByteRange {
        ByteRange::new(GuestAddr::new(start), length).unwrap()
    }

    #[test]
    fn shared_write_intent_roundtrip() {
        let e = Effect::SharedWriteIntent {
            range: range(0x1000, 4),
            bytes: WritePayload::new(vec![0xde, 0xad, 0xbe, 0xef]),
            ordering: PriorityClass::Normal,
            source: UnitId::new(2),
            source_time: GuestTicks::new(100),
        };
        let expected = Effect::SharedWriteIntent {
            range: range(0x1000, 4),
            bytes: WritePayload::new(vec![0xde, 0xad, 0xbe, 0xef]),
            ordering: PriorityClass::Normal,
            source: UnitId::new(2),
            source_time: GuestTicks::new(100),
        };
        assert_eq!(e, expected);
        assert_eq!(e.clone(), e);
    }

    #[test]
    fn mailbox_send_equality_and_distinct() {
        let a = Effect::MailboxSend {
            mailbox: MailboxId::new(1),
            message: MailboxMessage::new(7),
            source: UnitId::new(0),
        };
        let b = Effect::MailboxSend {
            mailbox: MailboxId::new(1),
            message: MailboxMessage::new(7),
            source: UnitId::new(0),
        };
        let c = Effect::MailboxSend {
            mailbox: MailboxId::new(1),
            message: MailboxMessage::new(8),
            source: UnitId::new(0),
        };
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn mailbox_receive_attempt_constructs() {
        let e = Effect::MailboxReceiveAttempt {
            mailbox: MailboxId::new(2),
            source: UnitId::new(3),
        };
        assert_eq!(e, e.clone());
    }

    #[test]
    fn dma_enqueue_carries_request() {
        let req = DmaRequest::new(
            DmaDirection::Put,
            range(0x100, 0x40),
            range(0x9000, 0x40),
            UnitId::new(5),
        )
        .unwrap();
        let e = Effect::DmaEnqueue {
            request: req,
            payload: None,
        };
        let expected = Effect::DmaEnqueue {
            request: req,
            payload: None,
        };
        assert_eq!(e, expected);
        assert_eq!(req.length(), 0x40);
        assert_eq!(req.issuer(), UnitId::new(5));
    }

    #[test]
    fn wait_on_event_each_target() {
        let m = Effect::WaitOnEvent {
            target: WaitTarget::Mailbox(MailboxId::new(1)),
            source: UnitId::new(0),
        };
        let s = Effect::WaitOnEvent {
            target: WaitTarget::Signal(SignalId::new(1)),
            source: UnitId::new(0),
        };
        let b = Effect::WaitOnEvent {
            target: WaitTarget::Barrier(BarrierId::new(1)),
            source: UnitId::new(0),
        };
        assert_ne!(m, s);
        assert_ne!(s, b);
        assert_ne!(m, b);
    }

    #[test]
    fn wake_unit_distinguishes_target_and_source() {
        let a = Effect::WakeUnit {
            target: UnitId::new(1),
            source: UnitId::new(2),
        };
        let b = Effect::WakeUnit {
            target: UnitId::new(2),
            source: UnitId::new(1),
        };
        assert_ne!(a, b);
    }

    #[test]
    fn signal_update_or_value() {
        let e = Effect::SignalUpdate {
            signal: SignalId::new(1),
            value: 0b0001_0000,
            source: UnitId::new(0),
        };
        let expected = Effect::SignalUpdate {
            signal: SignalId::new(1),
            value: 0x10,
            source: UnitId::new(0),
        };
        assert_eq!(e, expected);
    }

    #[test]
    fn fault_raised_validation_vs_guest() {
        let v = Effect::FaultRaised {
            kind: FaultKind::Validation,
            source: UnitId::new(0),
        };
        let g = Effect::FaultRaised {
            kind: FaultKind::Guest(42),
            source: UnitId::new(0),
        };
        assert_ne!(v, g);
    }

    #[test]
    fn trace_marker_roundtrip() {
        let e = Effect::TraceMarker {
            marker: 0xcafe,
            source: UnitId::new(9),
        };
        let expected = Effect::TraceMarker {
            marker: 0xcafe,
            source: UnitId::new(9),
        };
        assert_eq!(e, expected);
    }

    #[test]
    fn variants_are_distinct() {
        let mb = Effect::MailboxReceiveAttempt {
            mailbox: MailboxId::new(0),
            source: UnitId::new(0),
        };
        let wake = Effect::WakeUnit {
            target: UnitId::new(0),
            source: UnitId::new(0),
        };
        assert_ne!(mb, wake);
    }

    #[test]
    fn reservation_acquire_roundtrip() {
        let a = Effect::ReservationAcquire {
            line_addr: 0x1040,
            source: UnitId::new(3),
        };
        let b = Effect::ReservationAcquire {
            line_addr: 0x1040,
            source: UnitId::new(3),
        };
        let c = Effect::ReservationAcquire {
            line_addr: 0x1080,
            source: UnitId::new(3),
        };
        let d = Effect::ReservationAcquire {
            line_addr: 0x1040,
            source: UnitId::new(4),
        };
        assert_eq!(a, b);
        assert_ne!(a, c);
        assert_ne!(a, d);
    }

    #[test]
    fn conditional_store_roundtrip() {
        let e = Effect::ConditionalStore {
            range: range(0x1000, 4),
            bytes: WritePayload::new(vec![0xde, 0xad, 0xbe, 0xef]),
            ordering: PriorityClass::Normal,
            source: UnitId::new(2),
            source_time: GuestTicks::new(100),
        };
        let expected = Effect::ConditionalStore {
            range: range(0x1000, 4),
            bytes: WritePayload::new(vec![0xde, 0xad, 0xbe, 0xef]),
            ordering: PriorityClass::Normal,
            source: UnitId::new(2),
            source_time: GuestTicks::new(100),
        };
        assert_eq!(e, expected);
        assert_eq!(e.clone(), e);
    }

    #[test]
    fn conditional_store_distinguishes_source() {
        let a = Effect::ConditionalStore {
            range: range(0x1000, 4),
            bytes: WritePayload::new(vec![0; 4]),
            ordering: PriorityClass::Normal,
            source: UnitId::new(1),
            source_time: GuestTicks::new(0),
        };
        let b = Effect::ConditionalStore {
            range: range(0x1000, 4),
            bytes: WritePayload::new(vec![0; 4]),
            ordering: PriorityClass::Normal,
            source: UnitId::new(2),
            source_time: GuestTicks::new(0),
        };
        assert_ne!(a, b);
    }

    #[test]
    fn rsx_label_write_roundtrip() {
        let e = Effect::RsxLabelWrite {
            offset: 0x40,
            value: 0x1234_5678,
        };
        let expected = Effect::RsxLabelWrite {
            offset: 0x40,
            value: 0x1234_5678,
        };
        assert_eq!(e, expected);
        assert_eq!(e.clone(), e);
    }

    #[test]
    fn rsx_label_write_distinguishes_offset_and_value() {
        let a = Effect::RsxLabelWrite {
            offset: 0x40,
            value: 1,
        };
        let b = Effect::RsxLabelWrite {
            offset: 0x44,
            value: 1,
        };
        let c = Effect::RsxLabelWrite {
            offset: 0x40,
            value: 2,
        };
        assert_ne!(a, b);
        assert_ne!(a, c);
        assert_ne!(b, c);
    }

    #[test]
    fn rsx_flip_request_roundtrip() {
        let e = Effect::RsxFlipRequest { buffer_index: 1 };
        let expected = Effect::RsxFlipRequest { buffer_index: 1 };
        assert_eq!(e, expected);
        assert_eq!(e.clone(), e);
    }

    #[test]
    fn rsx_flip_request_distinguishes_buffer_index() {
        let a = Effect::RsxFlipRequest { buffer_index: 0 };
        let b = Effect::RsxFlipRequest { buffer_index: 1 };
        assert_ne!(a, b);
    }

    #[test]
    fn rsx_variants_distinct_from_existing_and_each_other() {
        let label = Effect::RsxLabelWrite {
            offset: 0,
            value: 0,
        };
        let flip = Effect::RsxFlipRequest { buffer_index: 0 };
        let write = Effect::SharedWriteIntent {
            range: range(0x1000, 4),
            bytes: WritePayload::new(vec![0; 4]),
            ordering: PriorityClass::Normal,
            source: UnitId::new(1),
            source_time: GuestTicks::new(0),
        };
        let acq = Effect::ReservationAcquire {
            line_addr: 0x1000,
            source: UnitId::new(1),
        };
        assert_ne!(label, flip);
        assert_ne!(label, write);
        assert_ne!(label, acq);
        assert_ne!(flip, write);
        assert_ne!(flip, acq);
    }

    #[test]
    fn reservation_variants_distinct_from_existing() {
        let acq = Effect::ReservationAcquire {
            line_addr: 0x1000,
            source: UnitId::new(1),
        };
        let write = Effect::SharedWriteIntent {
            range: range(0x1000, 4),
            bytes: WritePayload::new(vec![0; 4]),
            ordering: PriorityClass::Normal,
            source: UnitId::new(1),
            source_time: GuestTicks::new(0),
        };
        let cond = Effect::ConditionalStore {
            range: range(0x1000, 4),
            bytes: WritePayload::new(vec![0; 4]),
            ordering: PriorityClass::Normal,
            source: UnitId::new(1),
            source_time: GuestTicks::new(0),
        };
        assert_ne!(acq, write);
        assert_ne!(acq, cond);
        assert_ne!(write, cond);
    }
}
