//! The `Effect` enum -- the runtime's vocabulary for everything an
//! execution unit can ask the runtime to do on its behalf.
//!
//! `Effect` values are immutable packets. Units construct them and append
//! them to their step result; the runtime consumes them in emission order
//! during validation and commit. Emission order is preserved end-to-end,
//! even though commit batches are
//! atomic from the standpoint of guest visibility -- validation, conflict
//! diagnostics, fault attribution, and trace reconstruction all depend on
//! stable intra-step ordering.
//!
//! The variant set is exactly nine. Do not add
//! game-specific or instruction-specific variants here; architecture
//! crates layer their own wrapper effects on top of this set if needed.

use crate::payload::{FaultKind, MailboxMessage, WaitTarget, WritePayload};
use cellgov_dma::DmaRequest;
use cellgov_event::{PriorityClass, UnitId};
use cellgov_mem::ByteRange;
use cellgov_sync::{MailboxId, SignalId};
use cellgov_time::GuestTicks;

/// An immutable effect packet emitted by an execution unit.
///
/// `Effect` is `Clone + Eq + Debug`. It is intentionally not `Copy`
/// because [`Effect::SharedWriteIntent`] carries a heap-allocated
/// payload; cloning is the explicit way to duplicate an effect for
/// the trace.
///
/// The variant order below is part of the runtime's stable trace
/// contract: the binary trace format will rely on stable discriminants,
/// so reordering or inserting variants in the middle would break replay.
/// New variants must be appended at the end.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    /// Stage a write to globally visible memory. The write is not
    /// applied immediately; it flows through the commit pipeline and
    /// becomes visible at the next epoch boundary alongside the rest of
    /// its batch.
    SharedWriteIntent {
        /// Target byte range in the guest address space.
        range: ByteRange,
        /// Bytes to deposit into `range`. Length is checked against
        /// `range.length()` at commit validation time.
        bytes: WritePayload,
        /// Tier-2 ordering class used by conflict resolution when two
        /// units write to overlapping ranges in the same epoch.
        ordering: PriorityClass,
        /// The unit emitting this write.
        source: UnitId,
        /// The guest-time stamp at which this write becomes ordered.
        source_time: GuestTicks,
    },
    /// Send a message into a mailbox.
    MailboxSend {
        /// Destination mailbox.
        mailbox: MailboxId,
        /// Message word to deposit.
        message: MailboxMessage,
        /// The unit performing the send.
        source: UnitId,
    },
    /// Attempt to receive a message from a mailbox. The runtime decides
    /// whether the unit blocks (mailbox empty) or wakes immediately
    /// (message available); the unit just records its intent.
    MailboxReceiveAttempt {
        /// Mailbox to read from.
        mailbox: MailboxId,
        /// The unit performing the receive.
        source: UnitId,
    },
    /// Enqueue a DMA request for the runtime to model and complete.
    DmaEnqueue {
        /// The DMA request packet.
        request: DmaRequest,
        /// Inline payload for transfers from unit-private memory (e.g.,
        /// SPU local store). When present, the commit pipeline writes
        /// these bytes to the destination at completion time instead of
        /// reading from the source address in guest memory.
        payload: Option<Vec<u8>>,
    },
    /// Block this unit until the named event fires.
    WaitOnEvent {
        /// What the unit is waiting for.
        target: WaitTarget,
        /// The unit that is going to sleep.
        source: UnitId,
    },
    /// Wake another unit. Used for handing back roundtrip responses,
    /// releasing barrier participants, and any other unit-to-unit
    /// notification that does not need a sync primitive.
    WakeUnit {
        /// The unit to wake.
        target: UnitId,
        /// The unit performing the wake.
        source: UnitId,
    },
    /// Update a signal notification register. The `value` is OR-written
    /// into the register; the runtime applies the OR at commit time.
    SignalUpdate {
        /// Target signal register.
        signal: SignalId,
        /// Value to OR into the register.
        value: u32,
        /// The unit performing the update.
        source: UnitId,
    },
    /// Raise a fault. The runtime discards
    /// every effect emitted in this step (including effects that
    /// preceded `FaultRaised` in emission order) and routes the fault
    /// to the unit's fault-handling state.
    FaultRaised {
        /// Fault classification.
        kind: FaultKind,
        /// The unit raising the fault.
        source: UnitId,
    },
    /// Drop a debug breadcrumb into the trace. Has no semantic effect
    /// on the commit pipeline; exists so units and tests can correlate
    /// trace records with specific code paths.
    TraceMarker {
        /// Opaque marker value, recorded as-is in the trace.
        marker: u32,
        /// The unit emitting the marker.
        source: UnitId,
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
        // Sanity-check the carried request's fields.
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
        // Cross-variant inequality check: two effects with the same
        // source but different variant kinds must never compare equal.
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
}
