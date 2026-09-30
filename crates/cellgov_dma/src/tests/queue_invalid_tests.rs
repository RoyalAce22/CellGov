//! Invalid MFC commands in the queue: each holds a slot, and the queue
//! suspends its issuer when it reaches one.

use super::*;
use crate::command::{InvalidMfcCommand, MfcCommandError, MfcParameters};
use crate::request::{DmaDirection, DmaRequest};
use cellgov_mem::{ByteRange, GuestAddr};

const A: UnitId = UnitId::new(1);
const B: UnitId = UnitId::new(2);

fn put_at(time: u64, issuer: UnitId) -> DmaCompletion {
    let req = DmaRequest::new(
        DmaDirection::Put,
        ByteRange::new(GuestAddr::new(0x1000), 0x10).unwrap(),
        ByteRange::new(GuestAddr::new(0x9000), 0x10).unwrap(),
        issuer,
    )
    .unwrap()
    .with_tag_id(cellgov_ps3_abi::hw::spu::MfcTagId::new(2).unwrap());
    DmaCompletion::new(req, GuestTicks::new(time))
}

fn invalid(tag: u32, error: MfcCommandError) -> InvalidMfcCommand {
    InvalidMfcCommand {
        word: 0x20,
        params: MfcParameters {
            lsa: 0x100,
            eah: 0,
            eal: 0x2000,
            size: 3,
            tag,
        },
        error,
    }
}

#[test]
fn an_invalid_command_holds_a_slot_and_its_valid_tag() {
    let mut q = DmaQueue::new();
    q.enqueue_invalid(
        GuestTicks::new(10),
        A,
        invalid(5, MfcCommandError::SizeUnaligned(3)),
    );
    q.enqueue_invalid(
        GuestTicks::new(10),
        A,
        invalid(40, MfcCommandError::ReservedTagBits(40)),
    );
    assert_eq!(
        q.issuer_view(A),
        (2, 1 << 5),
        "a tag past 31 names no group"
    );
    assert_eq!(q.issuer_view(B), (0, 0));
    assert_eq!(q.len(), 2);
}

/// [CBEA p:113 s:9.1.1] an invalid command or parameter suspends SPU command queue processing.
#[test]
fn reaching_an_invalid_command_suspends_its_issuer_and_no_other() {
    let mut q = DmaQueue::new();
    q.enqueue(put_at(10, A), None);
    let command = invalid(5, MfcCommandError::SizeUnaligned(3));
    q.enqueue_invalid(GuestTicks::new(20), A, command);
    q.enqueue(put_at(30, A), None);
    q.enqueue(put_at(40, B), None);

    let due = q.process_due(GuestTicks::new(100));
    let completed: Vec<(u64, u64)> = due
        .completions
        .iter()
        .map(|(c, _)| (c.completion_time().raw(), c.issuer().raw()))
        .collect();
    assert_eq!(completed, [(10, 1), (40, 2)], "A's later put is held");
    assert_eq!(due.raised, [RaisedMfcCommand { issuer: A, command }]);
    assert!(q.suspended(A));
    assert!(!q.suspended(B));
    assert_eq!(
        q.issuer_view(A),
        (2, 1 << 5 | 1 << 2),
        "the command and the held put"
    );
    assert_eq!(q.next_event_time(), None, "nothing held ever happens");

    let again = q.process_due(GuestTicks::new(u64::MAX));
    assert!(again.completions.is_empty() && again.raised.is_empty());
    assert_eq!(q.len(), 2, "the drain leaves the command and the held put");
}

#[test]
fn an_invalid_command_is_reached_in_queue_order_with_the_transfers() {
    let mut q = DmaQueue::new();
    q.enqueue_invalid(
        GuestTicks::new(10),
        A,
        invalid(5, MfcCommandError::SizeUnaligned(3)),
    );
    q.enqueue(put_at(10, A), None);
    assert_eq!(q.next_event_time(), Some(GuestTicks::new(10)));
    let due = q.process_due(GuestTicks::new(10));
    assert_eq!(due.raised.len(), 1);
    assert!(
        due.completions.is_empty(),
        "the put enqueued after it is held"
    );
}

#[test]
fn an_invalid_command_and_its_raising_move_the_sync_partial() {
    let mut q = DmaQueue::new();
    q.enqueue(put_at(10, B), None);
    let transfers_only = q.sync_partial();
    q.enqueue_invalid(
        GuestTicks::new(20),
        A,
        invalid(5, MfcCommandError::SizeUnaligned(3)),
    );
    let queued = q.sync_partial();
    assert_ne!(queued, transfers_only);
    let _ = q.process_due(GuestTicks::new(20));
    assert_ne!(q.sync_partial(), queued, "raising changes a lane");
    assert_eq!(q.sync_partial(), q.sync_partial_from_scratch());
}

/// Two queues that differ only in when the queue reaches the command, or
/// only in a parameter the error does not name, raise at different times
/// or with different exceptions.
#[test]
fn the_sync_partial_names_the_time_and_every_parameter() {
    let partial = |time: u64, command: InvalidMfcCommand| {
        let mut q = DmaQueue::new();
        q.enqueue_invalid(GuestTicks::new(time), A, command);
        q.sync_partial()
    };
    let base = invalid(5, MfcCommandError::SizeUnaligned(3));
    let reference = partial(20, base);
    assert_ne!(partial(30, base), reference, "the time");
    let variants = [
        MfcParameters {
            lsa: 0x200,
            ..base.params
        },
        MfcParameters {
            eah: 1,
            ..base.params
        },
        MfcParameters {
            eal: 0x3000,
            ..base.params
        },
        MfcParameters {
            size: 5,
            ..base.params
        },
    ];
    for params in variants {
        assert_ne!(
            partial(20, InvalidMfcCommand { params, ..base }),
            reference,
            "{params:?}"
        );
    }
    // Tags 32 and 33 name no group and fail with one error code.
    let tag = |t| invalid(t, MfcCommandError::ReservedTagBits(32));
    assert_ne!(partial(20, tag(32)), partial(20, tag(33)), "the tag value");
}

#[test]
fn a_suspended_issuer_raises_no_second_command() {
    let mut q = DmaQueue::new();
    let first = invalid(5, MfcCommandError::SizeUnaligned(3));
    q.enqueue_invalid(GuestTicks::new(10), A, first);
    q.enqueue_invalid(
        GuestTicks::new(20),
        A,
        invalid(6, MfcCommandError::SizeUnaligned(3)),
    );
    let due = q.process_due(GuestTicks::new(100));
    assert_eq!(
        due.raised,
        [RaisedMfcCommand {
            issuer: A,
            command: first
        }]
    );
    assert_eq!(q.next_event_time(), None, "the second is held");
    assert!(q.process_due(GuestTicks::new(u64::MAX)).raised.is_empty());
}
