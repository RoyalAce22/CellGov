//! WritePayload inline-vs-heap storage plus MailboxMessage, WaitTarget, and FaultKind semantics.

use super::*;

#[test]
fn write_payload_basic() {
    let p = WritePayload::new(vec![1, 2, 3, 4]);
    assert_eq!(p.len(), 4);
    assert!(!p.is_empty());
    assert_eq!(p.bytes(), &[1, 2, 3, 4]);
}

#[test]
fn write_payload_empty_is_legal() {
    let p = WritePayload::new(vec![]);
    assert!(p.is_empty());
    assert_eq!(p.len(), 0);
}

#[test]
fn write_payload_equality() {
    let a = WritePayload::new(vec![1, 2, 3]);
    let b = WritePayload::new(vec![1, 2, 3]);
    let c = WritePayload::new(vec![1, 2, 4]);
    assert_eq!(a, b);
    assert_ne!(a, c);
}

#[test]
fn from_slice_matches_new() {
    let data = [0xDE, 0xAD, 0xBE, 0xEF];
    let a = WritePayload::new(data.to_vec());
    let b = WritePayload::from_slice(&data);
    assert_eq!(a, b);
    assert_eq!(b.bytes(), &data);
    assert_eq!(b.len(), 4);
}

#[test]
fn from_slice_empty() {
    let p = WritePayload::from_slice(&[]);
    assert!(p.is_empty());
    assert_eq!(p.len(), 0);
    assert_eq!(p.bytes(), &[]);
}

#[test]
fn inline_at_boundary() {
    let data = [0xAB; INLINE_CAP];
    let p = WritePayload::from_slice(&data);
    assert_eq!(p.len(), INLINE_CAP);
    assert_eq!(p.bytes(), &data);
    assert_eq!(p, WritePayload::new(data.to_vec()));
}

#[test]
fn heap_above_boundary() {
    let data = vec![0xCD; INLINE_CAP + 1];
    let p = WritePayload::from_slice(&data);
    assert_eq!(p.len(), INLINE_CAP + 1);
    assert_eq!(p.bytes(), data.as_slice());
    assert_eq!(p, WritePayload::new(data));
}

#[test]
fn clone_preserves_storage() {
    let inline = WritePayload::from_slice(&[1, 2, 3]);
    let heap = WritePayload::from_slice(&[0xFF; INLINE_CAP + 1]);
    assert_eq!(inline.clone(), inline);
    assert_eq!(heap.clone(), heap);
}

#[test]
fn mailbox_message_roundtrip() {
    assert_eq!(MailboxMessage::new(0xdead).raw(), 0xdead);
    assert_eq!(MailboxMessage::default(), MailboxMessage::new(0));
}

#[test]
fn mailbox_message_ordering() {
    assert!(MailboxMessage::new(1) < MailboxMessage::new(2));
}

#[test]
fn wait_target_distinct_variants() {
    let m = WaitTarget::Mailbox(MailboxId::new(1));
    let s = WaitTarget::Signal(SignalId::new(1));
    let b = WaitTarget::Barrier(BarrierId::new(1));
    assert_ne!(m, s);
    assert_ne!(s, b);
    assert_ne!(m, b);
    assert_eq!(m, WaitTarget::Mailbox(MailboxId::new(1)));
}

#[test]
fn wait_target_distinguishes_id() {
    assert_ne!(
        WaitTarget::Mailbox(MailboxId::new(1)),
        WaitTarget::Mailbox(MailboxId::new(2))
    );
}

#[test]
fn fault_kind_validation_equality() {
    assert_eq!(FaultKind::Validation, FaultKind::Validation);
    assert_ne!(FaultKind::Validation, FaultKind::Guest(0));
}

#[test]
fn fault_kind_guest_carries_code() {
    let a = FaultKind::Guest(7);
    let b = FaultKind::Guest(7);
    let c = FaultKind::Guest(8);
    assert_eq!(a, b);
    assert_ne!(a, c);
}
