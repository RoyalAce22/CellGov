//! DMA arguments the enqueue refuses by name rather than panicking at
//! completion.
//!
//! The completion reads the source out of committed space 0 and writes
//! the destination there, so both ends have to resolve at enqueue. An
//! end that does not is a refusal the caller can read, in the shape
//! every other bad DMA argument already takes.

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

#[test]
fn an_unmapped_dma_source_is_refused_at_enqueue() {
    let err = refusal_of(vec![enqueue(range(UNMAPPED, 4), range(0, 4), None)]);
    assert_eq!(err, CommitError::DmaSourceOutOfRange { effect_index: 0 });
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

/// An inline payload carries the bytes, so no source range is read and
/// none has to resolve.
#[test]
fn an_unmapped_source_with_an_inline_payload_is_accepted() {
    let mut bed = CommitTestBed::new(8);
    bed.units.register_with(DummyUnit::runnable);
    let (result, e) = step_with(
        YieldReason::BudgetExhausted,
        vec![enqueue(
            range(UNMAPPED, 4),
            range(0, 4),
            Some(vec![1, 2, 3, 4]),
        )],
    );
    let outcome = bed
        .process(&result, &e)
        .expect("the payload is the bytes, so the source is never read");
    assert_eq!(outcome.dma_enqueued, 1);
}

/// Every refusal marks the issuer, so it cannot poll a tag bit that
/// will never arrive.
#[test]
fn each_refused_enqueue_faults_the_issuer() {
    let shapes = [
        enqueue(range(UNMAPPED, 4), range(0, 4), None),
        enqueue(range(0, 4), range(4, 4), Some(vec![1, 2, 3])),
        enqueue(range(0, 4), range(UNMAPPED, 4), None),
        request(DmaDirection::Get, range(0, 4), range(4, 4), None),
    ];
    for shape in shapes {
        let (err, status) = refused(vec![shape]);
        assert_eq!(status, Some(UnitStatus::Faulted), "{err}");
    }
}

/// A `Get` names the local-store end as its destination, which the
/// completion would write into main memory.
#[test]
fn a_get_is_refused_by_name() {
    let err = refusal_of(vec![request(
        DmaDirection::Get,
        range(0, 4),
        range(4, 4),
        None,
    )]);
    assert_eq!(
        err,
        CommitError::DmaDirectionUnsupported { effect_index: 0 }
    );
}

/// The premise for the refusals above: the same enqueue with both ends
/// mapped reaches the queue.
#[test]
fn a_put_between_two_mapped_ends_is_accepted() {
    let mut bed = CommitTestBed::new(8);
    bed.units.register_with(DummyUnit::runnable);
    let (result, e) = step_with(
        YieldReason::BudgetExhausted,
        vec![enqueue(range(0, 4), range(4, 4), None)],
    );
    let outcome = bed.process(&result, &e).expect("both ends resolve");
    assert_eq!(outcome.dma_enqueued, 1);
}

/// Base of the zero-readable region [`bed_with_reserved_sources`] adds.
const ZERO_READABLE: u64 = 0x2000;

/// Base of the strict region [`bed_with_reserved_sources`] adds.
const STRICT: u64 = 0x3000;

fn bed_with_reserved_sources() -> CommitTestBed {
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

/// Validating the source reports no read of its own. The count exists
/// for the reads a run makes, and the transfer's own read happens at
/// completion -- after a batch this one may still refuse.
#[test]
fn validating_a_zero_readable_source_reports_no_provisional_read() {
    let mut bed = bed_with_reserved_sources();
    let (result, e) = step_with(
        YieldReason::BudgetExhausted,
        vec![enqueue(range(ZERO_READABLE, 4), range(0, 4), None)],
    );
    let outcome = bed
        .process(&result, &e)
        .expect("a zero-readable source resolves");
    assert_eq!(outcome.dma_enqueued, 1);
    assert_eq!(bed.memory().provisional_read_count(), 0);
}

/// A strict source resolves to a region, so it is not a range that
/// escapes them: the refusal carries the memory layer's own error.
#[test]
fn a_strict_reserved_source_is_refused_as_a_memory_error() {
    let mut bed = bed_with_reserved_sources();
    let (result, e) = step_with(
        YieldReason::BudgetExhausted,
        vec![enqueue(range(STRICT, 4), range(0, 4), None)],
    );
    let err = bed
        .process(&result, &e)
        .expect_err("reading a strict region faults");
    assert_eq!(
        err,
        CommitError::Memory(MemError::ReservedStrictRead {
            addr: STRICT,
            region: "strict",
        }),
    );
}
