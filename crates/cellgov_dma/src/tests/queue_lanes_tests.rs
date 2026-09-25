//! `DmaQueue` keeps its partial of the sync-state sum, and every field of
//! a queued completion reaches a lane.

use super::*;
use crate::request::{DmaDirection, DmaRequest};
use cellgov_event::UnitId;
use cellgov_mem::{ByteRange, GuestAddr};

fn range(start: u64, length: u64) -> ByteRange {
    ByteRange::new(GuestAddr::new(start), length).expect("range fits")
}

fn completion_at(time: u64, issuer: u64) -> DmaCompletion {
    let req = DmaRequest::new(
        DmaDirection::Put,
        range(0x1000, 0x10),
        range(0x9000, 0x10),
        UnitId::new(issuer),
    )
    .unwrap();
    DmaCompletion::new(req, GuestTicks::new(time))
}

fn partial_of(completion: DmaCompletion, payload: Option<Vec<u8>>) -> u128 {
    let mut q = DmaQueue::new();
    q.enqueue(completion, payload);
    assert_eq!(q.sync_partial(), q.sync_partial_from_scratch());
    q.sync_partial()
}

#[test]
fn sync_partial_distinguishes_each_field() {
    let request = |direction, source, destination, issuer| {
        DmaRequest::new(
            direction,
            range(source, 0x10),
            range(destination, 0x10),
            UnitId::new(issuer),
        )
        .unwrap()
    };
    let at = |req, time| DmaCompletion::new(req, GuestTicks::new(time));
    let base_req = request(DmaDirection::Put, 0x1000, 0x9000, 1);
    let base = partial_of(at(base_req, 100), None);
    let tag = cellgov_ps3_abi::hw::spu::MfcTagId::new(3).unwrap();
    let long = DmaRequest::new(
        DmaDirection::Put,
        range(0x1000, 0x20),
        range(0x9000, 0x20),
        UnitId::new(1),
    )
    .unwrap();
    let variants = [
        ("time", partial_of(at(base_req, 101), None)),
        (
            "direction",
            partial_of(at(request(DmaDirection::Get, 0x1000, 0x9000, 1), 100), None),
        ),
        (
            "source",
            partial_of(at(request(DmaDirection::Put, 0x1100, 0x9000, 1), 100), None),
        ),
        (
            "destination",
            partial_of(at(request(DmaDirection::Put, 0x1000, 0x9100, 1), 100), None),
        ),
        ("length", partial_of(at(long, 100), None)),
        (
            "issuer",
            partial_of(at(request(DmaDirection::Put, 0x1000, 0x9000, 2), 100), None),
        ),
        ("tag", partial_of(at(base_req.with_tag_id(tag), 100), None)),
        (
            "empty payload",
            partial_of(at(base_req, 100), Some(Vec::new())),
        ),
        ("payload", partial_of(at(base_req, 100), Some(vec![0xAB]))),
    ];
    for (what, partial) in variants {
        assert_ne!(partial, base, "{what} did not move the partial");
    }
    assert_ne!(
        partial_of(at(base_req, 100), Some(vec![0xAB])),
        partial_of(at(base_req, 100), Some(vec![0xAC])),
        "payload bytes",
    );
}

#[test]
fn sync_partial_follows_every_drain_path() {
    let mut q = DmaQueue::new();
    assert_eq!(q.sync_partial(), 0);
    q.enqueue(completion_at(50, 0), Some(vec![1, 2, 3]));
    q.enqueue(completion_at(100, 1), None);
    q.enqueue(completion_at(150, 2), None);
    let three = q.sync_partial();
    assert_eq!(three, q.sync_partial_from_scratch());
    let _ = q.pop_next();
    let two = q.sync_partial();
    assert_ne!(two, three);
    assert_eq!(two, q.sync_partial_from_scratch());
    assert_eq!(q.pop_due(GuestTicks::new(100)).len(), 1);
    assert_ne!(q.sync_partial(), two);
    assert_eq!(q.sync_partial(), q.sync_partial_from_scratch());
    let _ = q.pop_due(GuestTicks::new(u64::MAX));
    assert_eq!(q.sync_partial(), 0, "a drained queue has no lanes");
}
