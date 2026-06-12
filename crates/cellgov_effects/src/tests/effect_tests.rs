//! Effect variant construction, field-wise equality, and cross-variant distinctness.

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
