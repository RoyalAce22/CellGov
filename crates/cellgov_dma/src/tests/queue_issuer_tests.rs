//! Processing one issuer's queued commands whatever their time: the
//! other issuers' commands stay queued, and a refused command still
//! suspends the issuer.

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
        ByteRange::new(GuestAddr::new(0x9000 + time), 0x10).unwrap(),
        issuer,
    )
    .unwrap();
    DmaCompletion::new(req, GuestTicks::new(time))
}

fn times(due: &DueCommands) -> Vec<u64> {
    due.completions
        .iter()
        .map(|(c, _)| c.completion_time().raw())
        .collect()
}

#[test]
fn an_issuer_drain_takes_every_command_of_that_issuer_and_no_other() {
    let mut q = DmaQueue::new();
    q.enqueue(put_at(30, A), None);
    q.enqueue(put_at(10, B), None);
    q.enqueue(put_at(1000, A), None);
    q.enqueue(put_at(20, A), None);

    let due = q.process_issuer_translating(A, |_, _| None);
    assert_eq!(times(&due), [20, 30, 1000], "in completion order");
    assert!(due.raised.is_empty());
    assert_eq!(q.len(), 1);
    assert_eq!(q.peek().map(|c| c.issuer()), Some(B));
}

/// [CBEA p:113 s:9.1.1] an invalid command or parameter suspends SPU command queue processing.
#[test]
fn an_issuer_drain_stops_at_a_refused_command_and_keeps_the_rest() {
    let mut q = DmaQueue::new();
    q.enqueue(put_at(10, A), None);
    let command = InvalidMfcCommand {
        word: 0x20,
        params: MfcParameters {
            lsa: 0x100,
            eah: 0,
            eal: 0x2000,
            size: 3,
            tag: 5,
        },
        error: MfcCommandError::SizeUnaligned(3),
    };
    q.enqueue_invalid(GuestTicks::new(20), A, command);
    q.enqueue(put_at(30, A), None);

    let due = q.process_issuer_translating(A, |_, _| None);
    assert_eq!(times(&due), [10]);
    assert_eq!(due.raised, [RaisedMfcCommand { issuer: A, command }]);
    assert!(q.suspended(A));
    assert_eq!(q.len(), 2, "the refused command and the put behind it");
}
