//! The ordering floor a fence or barrier puts on a queued command.

use super::*;
use crate::completion::DmaCompletion;
use crate::request::{DmaDirection, DmaRequest, MfcOrdering};
use cellgov_mem::{ByteRange, GuestAddr};
use cellgov_ps3_abi::hw::spu::MfcTagId;

const A: UnitId = UnitId::new(1);
const B: UnitId = UnitId::new(2);

fn request(issuer: UnitId, tag: Option<u8>, ordering: MfcOrdering) -> DmaRequest {
    let range = ByteRange::new(GuestAddr::new(0x1000), 0x10).unwrap();
    let mut r = DmaRequest::new(DmaDirection::Put, range, range, issuer)
        .unwrap()
        .with_ordering(ordering);
    if let Some(tag) = tag {
        r = r.with_tag_id(MfcTagId::new(tag).unwrap());
    }
    r
}

fn queue(entries: &[(UnitId, Option<u8>, MfcOrdering, u64)]) -> DmaQueue {
    let mut q = DmaQueue::new();
    for &(issuer, tag, ordering, time) in entries {
        q.enqueue(
            DmaCompletion::new(request(issuer, tag, ordering), GuestTicks::new(time)),
            None,
        );
    }
    q
}

fn floor(q: &DmaQueue, issuer: UnitId, tag: Option<u8>, ordering: MfcOrdering) -> u64 {
    q.ordering_floor(&request(issuer, tag, ordering)).raw()
}

/// [CBEA p:69 s:7.9] a fence orders the command after every preceding command in its tag group, and no other.
#[test]
fn a_fence_waits_for_its_tag_group_alone() {
    let q = queue(&[
        (A, Some(1), MfcOrdering::None, 100),
        (A, Some(2), MfcOrdering::None, 300),
    ]);
    assert_eq!(floor(&q, A, Some(1), MfcOrdering::Fence), 100);
    assert_eq!(floor(&q, A, Some(3), MfcOrdering::Fence), 0);
    assert_eq!(
        floor(&q, A, Some(1), MfcOrdering::None),
        0,
        "no modifier, no wait"
    );
}

/// [CBEA p:69 s:7.9] a barrier orders the command and every later command of its tag group after the preceding commands of the group.
#[test]
fn a_queued_tag_barrier_holds_every_later_command_of_its_group() {
    let q = queue(&[
        (A, Some(1), MfcOrdering::None, 100),
        (A, Some(1), MfcOrdering::TagBarrier, 100),
        (A, Some(2), MfcOrdering::None, 300),
    ]);
    assert_eq!(floor(&q, A, Some(1), MfcOrdering::None), 100);
    assert_eq!(floor(&q, A, Some(2), MfcOrdering::None), 0, "another group");
}

#[test]
fn a_tag_barrier_holds_only_what_preceded_it() {
    let q = queue(&[
        (A, Some(1), MfcOrdering::TagBarrier, 50),
        (A, Some(1), MfcOrdering::None, 400),
    ]);
    assert_eq!(floor(&q, A, Some(1), MfcOrdering::None), 0);
}

/// [CBEA p:308 s:Appendix D Table D-4] the barrier command orders every preceding command before every following one in the queue.
#[test]
fn a_barrier_command_waits_for_every_earlier_command() {
    let q = queue(&[
        (A, Some(1), MfcOrdering::None, 100),
        (A, Some(2), MfcOrdering::None, 300),
    ]);
    assert_eq!(
        floor(&q, A, Some(5), MfcOrdering::QueueBarrier),
        300,
        "whatever their tag"
    );
}

/// [CBEA p:72 s:7.9.3] subsequent commands begin when the barrier command completes.
#[test]
fn a_queued_barrier_command_holds_every_later_command_behind_itself() {
    let q = queue(&[
        (A, Some(1), MfcOrdering::None, 100),
        (A, Some(5), MfcOrdering::QueueBarrier, 400),
    ]);
    assert_eq!(floor(&q, A, Some(3), MfcOrdering::None), 400);
}

/// Each SPU has its own queue, so another issuer's ordering binds nothing.
#[test]
fn another_issuers_commands_bind_nothing() {
    let q = queue(&[
        (A, Some(1), MfcOrdering::None, 100),
        (A, None, MfcOrdering::QueueBarrier, 100),
    ]);
    assert_eq!(floor(&q, B, Some(1), MfcOrdering::Fence), 0);
    assert_eq!(floor(&q, B, Some(1), MfcOrdering::None), 0);
}

#[test]
fn the_ordering_is_sync_state() {
    let plain = queue(&[(A, Some(1), MfcOrdering::None, 100)]);
    let fenced = queue(&[(A, Some(1), MfcOrdering::Fence, 100)]);
    assert_ne!(plain.sync_partial(), fenced.sync_partial());
    assert_eq!(fenced.sync_partial(), fenced.sync_partial_from_scratch());
}
