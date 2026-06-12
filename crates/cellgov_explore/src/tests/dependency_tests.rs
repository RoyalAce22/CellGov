//! Step-footprint conflict detection: overlapping versus disjoint writes and mailbox send/receive pairs.

use super::*;
use cellgov_dma::{DmaDirection, DmaRequest};
use cellgov_effects::payload::{MailboxMessage, WritePayload};
use cellgov_event::{PriorityClass, UnitId};
use cellgov_mem::GuestAddr;
use cellgov_time::GuestTicks;

fn range(start: u64, len: u64) -> ByteRange {
    ByteRange::new(GuestAddr::new(start), len).unwrap()
}

#[test]
fn overlapping_writes_conflict() {
    let a = StepFootprint::from_effects(&[Effect::SharedWriteIntent {
        range: range(0, 8),
        bytes: WritePayload::new(vec![0; 8]),
        ordering: PriorityClass::Normal,
        source: UnitId::new(0),
        source_time: GuestTicks::new(0),
    }]);
    let b = StepFootprint::from_effects(&[Effect::SharedWriteIntent {
        range: range(4, 8),
        bytes: WritePayload::new(vec![0; 8]),
        ordering: PriorityClass::Normal,
        source: UnitId::new(1),
        source_time: GuestTicks::new(0),
    }]);
    assert!(a.conflicts(&b));
}

#[test]
fn disjoint_writes_are_independent() {
    let a = StepFootprint::from_effects(&[Effect::SharedWriteIntent {
        range: range(0, 4),
        bytes: WritePayload::new(vec![0; 4]),
        ordering: PriorityClass::Normal,
        source: UnitId::new(0),
        source_time: GuestTicks::new(0),
    }]);
    let b = StepFootprint::from_effects(&[Effect::SharedWriteIntent {
        range: range(8, 4),
        bytes: WritePayload::new(vec![0; 4]),
        ordering: PriorityClass::Normal,
        source: UnitId::new(1),
        source_time: GuestTicks::new(0),
    }]);
    assert!(!a.conflicts(&b));
}

#[test]
fn send_receive_same_mailbox_conflicts() {
    let a = StepFootprint::from_effects(&[Effect::MailboxSend {
        mailbox: MailboxId::new(1),
        message: MailboxMessage::new(42),
        source: UnitId::new(0),
    }]);
    let b = StepFootprint::from_effects(&[Effect::MailboxReceiveAttempt {
        mailbox: MailboxId::new(1),
        source: UnitId::new(1),
    }]);
    assert!(a.conflicts(&b));
}

#[test]
fn send_receive_different_mailbox_independent() {
    let a = StepFootprint::from_effects(&[Effect::MailboxSend {
        mailbox: MailboxId::new(1),
        message: MailboxMessage::new(42),
        source: UnitId::new(0),
    }]);
    let b = StepFootprint::from_effects(&[Effect::MailboxReceiveAttempt {
        mailbox: MailboxId::new(2),
        source: UnitId::new(1),
    }]);
    assert!(!a.conflicts(&b));
}

#[test]
fn two_sends_same_mailbox_conflict() {
    let a = StepFootprint::from_effects(&[Effect::MailboxSend {
        mailbox: MailboxId::new(1),
        message: MailboxMessage::new(1),
        source: UnitId::new(0),
    }]);
    let b = StepFootprint::from_effects(&[Effect::MailboxSend {
        mailbox: MailboxId::new(1),
        message: MailboxMessage::new(2),
        source: UnitId::new(1),
    }]);
    assert!(a.conflicts(&b));
}

#[test]
fn signal_update_same_signal_conflicts() {
    let a = StepFootprint::from_effects(&[Effect::SignalUpdate {
        signal: SignalId::new(1),
        value: 0x1,
        source: UnitId::new(0),
    }]);
    let b = StepFootprint::from_effects(&[Effect::SignalUpdate {
        signal: SignalId::new(1),
        value: 0x2,
        source: UnitId::new(1),
    }]);
    assert!(a.conflicts(&b));
}

#[test]
fn signal_update_vs_wait_conflicts() {
    let a = StepFootprint::from_effects(&[Effect::SignalUpdate {
        signal: SignalId::new(1),
        value: 0x1,
        source: UnitId::new(0),
    }]);
    let b = StepFootprint::from_effects(&[Effect::WaitOnEvent {
        target: cellgov_effects::WaitTarget::Signal(SignalId::new(1)),
        source: UnitId::new(1),
    }]);
    assert!(a.conflicts(&b));
}

#[test]
fn signal_update_different_signal_independent() {
    let a = StepFootprint::from_effects(&[Effect::SignalUpdate {
        signal: SignalId::new(1),
        value: 0x1,
        source: UnitId::new(0),
    }]);
    let b = StepFootprint::from_effects(&[Effect::SignalUpdate {
        signal: SignalId::new(2),
        value: 0x2,
        source: UnitId::new(1),
    }]);
    assert!(!a.conflicts(&b));
}

#[test]
fn dma_overlapping_destination_conflicts() {
    let req_a = DmaRequest::new(
        DmaDirection::Put,
        range(0x100, 0x40),
        range(0x1000, 0x40),
        UnitId::new(0),
    )
    .unwrap();
    let req_b = DmaRequest::new(
        DmaDirection::Put,
        range(0x200, 0x40),
        range(0x1020, 0x40),
        UnitId::new(1),
    )
    .unwrap();
    let a = StepFootprint::from_effects(&[Effect::DmaEnqueue {
        request: req_a,
        payload: None,
    }]);
    let b = StepFootprint::from_effects(&[Effect::DmaEnqueue {
        request: req_b,
        payload: None,
    }]);
    assert!(a.conflicts(&b));
}

#[test]
fn write_vs_dma_overlapping_conflicts() {
    let req = DmaRequest::new(
        DmaDirection::Put,
        range(0x100, 0x40),
        range(0, 0x40),
        UnitId::new(1),
    )
    .unwrap();
    let a = StepFootprint::from_effects(&[Effect::SharedWriteIntent {
        range: range(0x10, 4),
        bytes: WritePayload::new(vec![0; 4]),
        ordering: PriorityClass::Normal,
        source: UnitId::new(0),
        source_time: GuestTicks::new(0),
    }]);
    let b = StepFootprint::from_effects(&[Effect::DmaEnqueue {
        request: req,
        payload: None,
    }]);
    assert!(a.conflicts(&b));
}

#[test]
fn wake_vs_wait_conflicts() {
    let a = StepFootprint::from_effects(&[Effect::WakeUnit {
        target: UnitId::new(2),
        source: UnitId::new(0),
    }]);
    let b = StepFootprint::from_effects(&[Effect::WaitOnEvent {
        target: cellgov_effects::WaitTarget::Mailbox(MailboxId::new(1)),
        source: UnitId::new(1),
    }]);
    assert!(a.conflicts(&b));
}

#[test]
fn wait_vs_wake_conflicts_symmetric() {
    let a = StepFootprint::from_effects(&[Effect::WaitOnEvent {
        target: cellgov_effects::WaitTarget::Mailbox(MailboxId::new(1)),
        source: UnitId::new(0),
    }]);
    let b = StepFootprint::from_effects(&[Effect::WakeUnit {
        target: UnitId::new(3),
        source: UnitId::new(1),
    }]);
    assert!(
        a.conflicts(&b),
        "wait vs wake should conflict symmetrically"
    );
}

#[test]
fn both_wait_same_barrier_conflicts() {
    let a = StepFootprint::from_effects(&[Effect::WaitOnEvent {
        target: cellgov_effects::WaitTarget::Barrier(BarrierId::new(1)),
        source: UnitId::new(0),
    }]);
    let b = StepFootprint::from_effects(&[Effect::WaitOnEvent {
        target: cellgov_effects::WaitTarget::Barrier(BarrierId::new(1)),
        source: UnitId::new(1),
    }]);
    assert!(a.conflicts(&b));
}

#[test]
fn empty_footprints_are_independent() {
    let a = StepFootprint::default();
    let b = StepFootprint::default();
    assert!(!a.conflicts(&b));
}

#[test]
fn trace_marker_only_is_local() {
    let fp = StepFootprint::from_effects(&[Effect::TraceMarker {
        marker: 0xCAFE,
        source: UnitId::new(0),
    }]);
    assert!(fp.is_local_only());
}

#[test]
fn fault_is_ignored() {
    let fp = StepFootprint::from_effects(&[Effect::FaultRaised {
        kind: cellgov_effects::FaultKind::Validation,
        source: UnitId::new(0),
    }]);
    assert!(fp.is_local_only());
}
