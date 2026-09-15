//! Step-footprint conflict detection.

use super::*;
use cellgov_dma::{DmaDirection, DmaRequest};
use cellgov_effects::payload::{MailboxMessage, WritePayload};
use cellgov_event::{PriorityClass, UnitId};
use cellgov_mem::GuestAddr;
use cellgov_time::GuestTicks;

/// Highest 128-byte-aligned line the address space holds.
const TOP_LINE: u64 = !(RESERVATION_LINE_BYTES - 1);

fn range(start: u64, len: u64) -> ByteRange {
    ByteRange::new(GuestAddr::new(start), len).unwrap()
}

#[test]
fn overlapping_writes_conflict() {
    let a = StepFootprint::from_effects(&[Effect::shared_write(
        range(0, 8),
        WritePayload::new(vec![0; 8]),
        UnitId::new(0),
        GuestTicks::new(0),
    )]);
    let b = StepFootprint::from_effects(&[Effect::shared_write(
        range(4, 8),
        WritePayload::new(vec![0; 8]),
        UnitId::new(1),
        GuestTicks::new(0),
    )]);
    assert!(a.conflicts(&b));
}

#[test]
fn disjoint_writes_are_independent() {
    let a = StepFootprint::from_effects(&[Effect::shared_write(
        range(0, 4),
        WritePayload::new(vec![0; 4]),
        UnitId::new(0),
        GuestTicks::new(0),
    )]);
    let b = StepFootprint::from_effects(&[Effect::shared_write(
        range(8, 4),
        WritePayload::new(vec![0; 4]),
        UnitId::new(1),
        GuestTicks::new(0),
    )]);
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
fn two_receives_same_mailbox_conflict() {
    let a = StepFootprint::from_effects(&[Effect::MailboxReceiveAttempt {
        mailbox: MailboxId::new(1),
        source: UnitId::new(0),
    }]);
    let b = StepFootprint::from_effects(&[Effect::MailboxReceiveAttempt {
        mailbox: MailboxId::new(1),
        source: UnitId::new(1),
    }]);
    assert!(a.conflicts(&b));
    assert!(b.conflicts(&a));
}

#[test]
fn two_receives_different_mailboxes_are_independent() {
    let a = StepFootprint::from_effects(&[Effect::MailboxReceiveAttempt {
        mailbox: MailboxId::new(1),
        source: UnitId::new(0),
    }]);
    let b = StepFootprint::from_effects(&[Effect::MailboxReceiveAttempt {
        mailbox: MailboxId::new(2),
        source: UnitId::new(1),
    }]);
    assert!(!a.conflicts(&b));
    assert!(!b.conflicts(&a));
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
    let a = StepFootprint::from_effects(&[Effect::shared_write(
        range(0x10, 4),
        WritePayload::new(vec![0; 4]),
        UnitId::new(0),
        GuestTicks::new(0),
    )]);
    let b = StepFootprint::from_effects(&[Effect::DmaEnqueue {
        request: req,
        payload: None,
    }]);
    assert!(a.conflicts(&b));
}

#[test]
fn dma_over_reserved_line_conflicts_without_byte_overlap() {
    // Reservation on line 0x1000 with a conditional store touching
    // only bytes 0x1000..0x1008.
    let a = StepFootprint::from_effects(&[
        Effect::ReservationAcquire {
            line_addr: 0x1000,
            source: UnitId::new(0),
        },
        Effect::ConditionalStore {
            range: range(0x1000, 8),
            bytes: WritePayload::new(vec![0; 8]),
            ordering: PriorityClass::Normal,
            source: UnitId::new(0),
            source_time: GuestTicks::new(0),
        },
    ]);
    // DMA put landing at 0x1040..0x1080: inside line 0x1000 but
    // disjoint from the store's bytes. Completion clears the
    // reservation, so the pair is order-dependent.
    let req = DmaRequest::new(
        DmaDirection::Put,
        range(0x8000, 0x40),
        range(0x1040, 0x40),
        UnitId::new(1),
    )
    .unwrap();
    let b = StepFootprint::from_effects(&[Effect::DmaEnqueue {
        request: req,
        payload: None,
    }]);
    assert!(a.conflicts(&b));
    assert!(b.conflicts(&a));
}

#[test]
fn a_wake_conflicts_with_the_wait_it_enables() {
    let waiter = UnitId::new(1);
    let a = StepFootprint::from_effects(&[Effect::WakeUnit {
        target: waiter,
        source: UnitId::new(0),
    }]);
    let b = StepFootprint::from_effects(&[Effect::WaitOnEvent {
        target: cellgov_effects::WaitTarget::Mailbox(MailboxId::new(1)),
        source: waiter,
    }]);
    assert!(a.conflicts(&b));
    assert!(b.conflicts(&a), "the pair conflicts from either side");
}

#[test]
fn a_wake_of_another_unit_is_independent_of_a_wait() {
    let a = StepFootprint::from_effects(&[Effect::WakeUnit {
        target: UnitId::new(2),
        source: UnitId::new(0),
    }]);
    let b = StepFootprint::from_effects(&[Effect::WaitOnEvent {
        target: cellgov_effects::WaitTarget::Mailbox(MailboxId::new(1)),
        source: UnitId::new(1),
    }]);
    assert!(!a.conflicts(&b));
    assert!(!b.conflicts(&a));
}

#[test]
fn the_wait_target_does_not_decide_the_pairing() {
    let waiter = UnitId::new(1);
    let wake = StepFootprint::from_effects(&[Effect::WakeUnit {
        target: waiter,
        source: UnitId::new(0),
    }]);
    for target in [
        cellgov_effects::WaitTarget::Mailbox(MailboxId::new(4)),
        cellgov_effects::WaitTarget::Signal(SignalId::new(4)),
        cellgov_effects::WaitTarget::Barrier(BarrierId::new(4)),
    ] {
        let wait = StepFootprint::from_effects(&[Effect::WaitOnEvent {
            target,
            source: waiter,
        }]);
        assert!(wake.conflicts(&wait), "{target:?}");
    }
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
fn different_barriers_are_independent() {
    let a = StepFootprint::from_effects(&[Effect::WaitOnEvent {
        target: cellgov_effects::WaitTarget::Barrier(BarrierId::new(1)),
        source: UnitId::new(0),
    }]);
    let b = StepFootprint::from_effects(&[Effect::WaitOnEvent {
        target: cellgov_effects::WaitTarget::Barrier(BarrierId::new(2)),
        source: UnitId::new(1),
    }]);
    assert!(!a.conflicts(&b));
    assert!(!b.conflicts(&a));
}

#[test]
fn the_top_line_ends_at_the_last_byte_without_saturating() {
    let reserver = StepFootprint::from_effects(&[Effect::ReservationAcquire {
        line_addr: u64::MAX,
        source: UnitId::new(0),
    }]);
    assert_eq!(reserver.reservation_lines, vec![TOP_LINE]);
    // The line comparison adds the granule to the masked address, so
    // assert on the value the mask produced.
    assert_eq!(
        reserver.reservation_lines[0].checked_add(RESERVATION_LINE_BYTES - 1),
        Some(u64::MAX),
    );
}

/// The highest write a `ByteRange` can represent ends one byte short
/// of saturation, and it still covers the top line.
#[test]
fn a_write_over_the_top_line_covers_it() {
    let reserver = StepFootprint::from_effects(&[Effect::ReservationAcquire {
        line_addr: TOP_LINE,
        source: UnitId::new(0),
    }]);
    let last_byte = StepFootprint::from_effects(&[Effect::shared_write(
        range(u64::MAX - 1, 1),
        WritePayload::new(vec![0; 1]),
        UnitId::new(1),
        GuestTicks::new(0),
    )]);
    assert!(reserver.conflicts(&last_byte));

    // One byte below the line still misses it.
    let below = StepFootprint::from_effects(&[Effect::shared_write(
        range(TOP_LINE - 1, 1),
        WritePayload::new(vec![0; 1]),
        UnitId::new(1),
        GuestTicks::new(0),
    )]);
    assert!(!reserver.conflicts(&below));
}

/// Pins the first bullet of `write_covers_any_line`: no wrapped range
/// reaches the line comparison.
#[test]
fn a_write_that_would_wrap_is_unrepresentable() {
    assert!(ByteRange::new(GuestAddr::new(u64::MAX), 1).is_none());
    assert!(ByteRange::new(GuestAddr::new(u64::MAX - 1), 1).is_some());
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
