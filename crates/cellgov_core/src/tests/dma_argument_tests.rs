//! DMA arguments the enqueue refuses by name, and the ones it leaves to
//! the queue.
//!
//! The pipeline refuses an enqueue whose shape is wrong:
//!
//! - a payload of the wrong length;
//! - a get that carries bytes.
//!
//! It checks no main-storage address. The queue raises an address that
//! does not translate when it reaches the transfer.

use super::*;
use crate::commit::tests::{range, step_with, CommitTestBed, DummyUnit};
use cellgov_dma::{DmaDirection, DmaRequest};
use cellgov_mem::{ByteRange, PageSize, Region, RegionAccess};

/// Outside every region any bed in this file builds.
const UNMAPPED: u64 = 4096;

fn enqueue(source: ByteRange, destination: ByteRange, payload: Option<Vec<u8>>) -> Effect {
    request(DmaDirection::Put, source, destination, payload)
}

fn request(
    direction: DmaDirection,
    source: ByteRange,
    destination: ByteRange,
    payload: Option<Vec<u8>>,
) -> Effect {
    Effect::DmaEnqueue {
        request: DmaRequest::new(direction, source, destination, UnitId::new(0))
            .expect("the two ends are the same length"),
        payload,
    }
}

fn refusal_of(effects: Vec<Effect>) -> CommitError {
    refused(effects).0
}

/// The refusal, and the issuer's status once the batch is refused.
fn refused(effects: Vec<Effect>) -> (CommitError, Option<UnitStatus>) {
    let mut bed = CommitTestBed::new(8);
    let issuer = bed.units.register_with(DummyUnit::runnable);
    let (result, e) = step_with(YieldReason::BudgetExhausted, effects);
    let err = bed
        .process(&result, &e)
        .expect_err("the pipeline refuses this enqueue");
    (err, bed.units.effective_status(issuer))
}

/// The completion writes the payload over the whole destination, so any
/// other length reaches the memory layer as a mismatch it has no
/// refusal for.
#[test]
fn a_payload_that_is_not_the_destination_length_is_refused() {
    for bytes in [vec![1, 2, 3], vec![1, 2, 3, 4, 5]] {
        let len = bytes.len();
        let err = refusal_of(vec![enqueue(range(0, 4), range(4, 4), Some(bytes))]);
        assert_eq!(
            err,
            CommitError::DmaPayloadLengthMismatch { effect_index: 0 },
            "payload of {len} bytes against a 4-byte destination",
        );
    }
}

/// Every refusal marks the issuer, so it cannot poll a tag bit that
/// will never arrive.
#[test]
fn each_refused_enqueue_faults_the_issuer() {
    let shapes = [
        enqueue(range(0, 4), range(4, 4), Some(vec![1, 2, 3])),
        request(
            DmaDirection::Get,
            range(0, 4),
            range(4, 4),
            Some(vec![1, 2, 3, 4]),
        ),
    ];
    for shape in shapes {
        let (err, status) = refused(vec![shape]);
        assert_eq!(status, Some(UnitStatus::Faulted), "{err}");
    }
}

/// A get reads its source when it completes, so an inline payload is
/// refused by name.
#[test]
fn a_get_with_an_inline_payload_is_refused_by_name() {
    let err = refusal_of(vec![request(
        DmaDirection::Get,
        range(0, 4),
        range(4, 4),
        Some(vec![1, 2, 3, 4]),
    )]);
    assert_eq!(err, CommitError::DmaGetWithPayload { effect_index: 0 });
}

/// Base of the zero-readable region [`bed_with_reserved_regions`] adds.
const ZERO_READABLE: u64 = 0x2000;

/// Base of the strict region [`bed_with_reserved_regions`] adds.
const STRICT: u64 = 0x3000;

fn bed_with_reserved_regions() -> CommitTestBed {
    let mem = GuestMemory::from_regions(vec![
        Region::new(0, 8, "main", PageSize::Page64K),
        Region::with_access(
            ZERO_READABLE,
            8,
            "zero_readable",
            PageSize::Page64K,
            RegionAccess::ReservedZeroReadable,
        ),
        Region::with_access(
            STRICT,
            8,
            "strict",
            PageSize::Page64K,
            RegionAccess::ReservedStrict,
        ),
    ])
    .expect("the three regions do not overlap");
    let mut bed = CommitTestBed::with_memory(mem);
    bed.units.register_with(DummyUnit::runnable);
    bed
}

/// The shapes cover each way an address does not translate:
///
/// - no region;
/// - a region the read may not use;
/// - a region the write may not use.
///
/// [CBEA p:118 s:9.1.6] the address's validity is checked asynchronous to the instruction stream, during the transfer.
#[test]
fn an_address_that_does_not_translate_reaches_the_queue() {
    let shapes = [
        enqueue(range(UNMAPPED, 4), range(0, 4), None),
        enqueue(range(0, 4), range(UNMAPPED, 4), None),
        enqueue(range(STRICT, 4), range(0, 4), None),
        enqueue(range(0, 4), range(ZERO_READABLE, 4), None),
        request(DmaDirection::Get, range(UNMAPPED, 4), range(0, 4), None),
    ];
    for shape in shapes {
        let mut bed = bed_with_reserved_regions();
        let (result, e) = step_with(YieldReason::BudgetExhausted, vec![shape.clone()]);
        let outcome = bed
            .process(&result, &e)
            .unwrap_or_else(|err| panic!("{shape:?} refused: {err}"));
        assert_eq!(outcome.dma_enqueued, 1, "{shape:?}");
        assert_ne!(
            bed.units.effective_status(UnitId::new(0)),
            Some(UnitStatus::Faulted),
            "{shape:?}"
        );
    }
}

/// The enqueue reads nothing: the transfer's own read happens when the
/// queue reaches it.
#[test]
fn an_enqueue_reports_no_provisional_read() {
    let mut bed = bed_with_reserved_regions();
    let (result, e) = step_with(
        YieldReason::BudgetExhausted,
        vec![enqueue(range(ZERO_READABLE, 4), range(0, 4), None)],
    );
    let outcome = bed.process(&result, &e).expect("the enqueue is accepted");
    assert_eq!(outcome.dma_enqueued, 1);
    assert_eq!(bed.memory().provisional_read_count(), 0);
}
