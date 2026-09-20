//! The validated host-write path: refusal parity with a unit write,
//! the reservation clear sweep, and the trace record it leaves.

use cellgov_effects::{Effect, WritePayload};
use cellgov_event::UnitId;
use cellgov_exec::{ExecutionStepResult, LocalDiagnostics, YieldReason};
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory, MemError, PageSize, Region, RegionAccess};
use cellgov_sync::ReservedLine;
use cellgov_time::{Budget, GuestTicks, InstructionCost};
use cellgov_trace::{HostWriter, TraceReader, TraceRecord};

use crate::commit::{CommitContext, CommitError};

use super::*;

const MAIN: u64 = 0;
const RSX: u64 = 0xC000_0000;

/// Space 0 with a writable region at `MAIN` and a reserved one at `RSX`.
fn build() -> Runtime {
    let memory = GuestMemory::from_regions(vec![
        Region::new(MAIN, 4096, "main", PageSize::Page64K),
        Region::with_access(
            RSX,
            256,
            "rsx",
            PageSize::Page64K,
            RegionAccess::ReservedZeroReadable,
        ),
    ])
    .expect("disjoint regions in ascending order");
    Runtime::new(memory, Budget::new(4), 100)
}

fn range(addr: u64, len: u64) -> ByteRange {
    ByteRange::new(GuestAddr::new(addr), len).expect("in-range test address")
}

fn host_writes(rt: &Runtime) -> Vec<TraceRecord> {
    TraceReader::new(rt.trace().bytes())
        .map(|r| r.expect("the runtime's own stream decodes"))
        .filter(|r| matches!(r, TraceRecord::HostWrite { .. }))
        .collect()
}

/// The error a unit's `SharedWriteIntent` for the same range dies with.
fn unit_write_error(rt: &mut Runtime, target: ByteRange) -> CommitError {
    let effects = vec![Effect::shared_write(
        target,
        WritePayload::new(vec![0xAB; target.length() as usize]),
        UnitId::new(0),
        GuestTicks::ZERO,
    )];
    let result = ExecutionStepResult {
        yield_reason: YieldReason::Finished,
        consumed_cost: InstructionCost::new(1),
        local_diagnostics: LocalDiagnostics::empty(),
        fault: None,
        syscall_args: None,
    };
    // The pipeline's own pre-validation passes a reserved region (it
    // checks containment alone), so the refusal comes from the staged
    // drain -- the same `validate_write` `apply_commit` runs.
    let mut rsx_label_writes = 0u64;
    let mut ctx = CommitContext {
        space: 0,
        memory: &mut rt.memory,
        dma_memory: None,
        units: &mut rt.registry,
        mailboxes: &mut rt.mailbox_registry,
        signals: &mut rt.signal_registry,
        dma_queue: &mut rt.dma_queue,
        dma_latency: rt.dma_latency.as_ref(),
        now: GuestTicks::ZERO,
        reservations: &mut rt.reservations,
        rsx_label_base: 0,
        rsx_flip: &mut rt.rsx_flip,
        rsx_label_writes_committed: &mut rsx_label_writes,
        tap: None,
    };
    let mut pipeline = crate::commit::CommitPipeline::new();
    pipeline
        .process(&result, &effects, &mut ctx)
        .expect_err("a write into the reserved region cannot commit")
}

#[test]
fn a_host_write_into_a_reserved_region_is_refused_like_a_unit_write() {
    let mut rt = build();
    let target = range(RSX, 4);

    let host_err = rt
        .host_write(
            HostWriter::Lv2Effect,
            AddressSpaceId::BOOT,
            target,
            &[0xAB; 4],
            None,
        )
        .expect_err("a host write into the reserved region cannot commit");

    assert_eq!(
        CommitError::Memory(host_err),
        unit_write_error(&mut rt, target),
        "the host path must refuse a reserved region with the same error a unit does",
    );
}

#[test]
fn a_refused_host_write_sweeps_nothing_and_traces_nothing() {
    let mut rt = build();
    rt.reservations_mut()
        .insert_or_replace(UnitId::new(1), ReservedLine::containing(RSX));

    let err = rt.host_write(
        HostWriter::RsxMirror,
        AddressSpaceId::BOOT,
        range(RSX, 4),
        &[0xAB; 4],
        None,
    );

    assert!(matches!(err, Err(MemError::ReservedWrite { .. })));
    assert_eq!(
        rt.reservations().get(UnitId::new(1)),
        Some(ReservedLine::containing(RSX)),
        "a write that never landed cannot invalidate a reservation",
    );
    assert!(host_writes(&rt).is_empty());
}

#[test]
fn an_accepted_host_write_clears_every_reservation_but_the_exempt_one() {
    let mut rt = build();
    let holder = UnitId::new(1);
    let exempt = UnitId::new(2);
    let elsewhere = UnitId::new(3);
    rt.reservations_mut()
        .insert_or_replace(holder, ReservedLine::containing(MAIN));
    rt.reservations_mut()
        .insert_or_replace(exempt, ReservedLine::containing(MAIN));
    rt.reservations_mut()
        .insert_or_replace(elsewhere, ReservedLine::containing(MAIN + 0x800));

    let cleared = rt
        .host_write(
            HostWriter::WakeContinuation,
            AddressSpaceId::BOOT,
            range(MAIN, 4),
            &[0xAB; 4],
            Some(exempt),
        )
        .expect("a writable region accepts the write");

    assert_eq!(cleared, 1);
    assert_eq!(rt.reservations().get(holder), None);
    assert_eq!(
        rt.reservations().get(exempt),
        Some(ReservedLine::containing(MAIN))
    );
    assert_eq!(
        rt.reservations().get(elsewhere),
        Some(ReservedLine::containing(MAIN + 0x800)),
        "the sweep covers the written range, not the whole space",
    );
}

#[test]
fn an_accepted_host_write_names_its_mechanism_in_the_trace() {
    let mut rt = build();
    rt.reservations_mut()
        .insert_or_replace(UnitId::new(1), ReservedLine::containing(MAIN));

    rt.host_write(
        HostWriter::DmaCompletion,
        AddressSpaceId::BOOT,
        range(MAIN, 8),
        &[0xAB; 8],
        None,
    )
    .expect("a writable region accepts the write");

    assert_eq!(
        host_writes(&rt),
        vec![TraceRecord::HostWrite {
            writer: HostWriter::DmaCompletion,
            space: AddressSpaceId::BOOT.raw(),
            addr: MAIN,
            len: 8,
            reservations_cleared: 1,
        }]
    );
}

#[test]
fn an_lv2_out_parameter_write_invalidates_another_units_reservation() {
    let mut rt = build();
    let caller = UnitId::new(0);
    let other = UnitId::new(1);
    rt.reservations_mut()
        .insert_or_replace(caller, ReservedLine::containing(MAIN));
    rt.reservations_mut()
        .insert_or_replace(other, ReservedLine::containing(MAIN));

    rt.apply_lv2_effects(
        &[Effect::shared_write(
            range(MAIN, 4),
            WritePayload::new(vec![0xAB; 4]),
            caller,
            GuestTicks::ZERO,
        )],
        AddressSpaceId::BOOT,
    );

    assert_eq!(rt.reservations().get(other), None);
    assert_eq!(
        rt.reservations().get(caller),
        Some(ReservedLine::containing(MAIN)),
        "the kernel writes on the caller's behalf, so the caller keeps its own line",
    );
}

#[test]
fn a_host_write_lands_in_the_named_space_not_the_boot_space() {
    let mut rt = build();
    let child = AddressSpaceId::new(1);
    rt.create_address_space(child).expect("space 1 is fresh");
    rt.space_memory_mut(child)
        .expect("the space was just created")
        .install_region(MAIN, 4096, "child_main", PageSize::Page64K)
        .expect("an empty space has no region to overlap");
    let boot_holder = UnitId::new(1);
    let child_holder = UnitId::new(2);
    rt.reservations_mut()
        .insert_or_replace(boot_holder, ReservedLine::containing(MAIN));
    rt.space_reservations_mut(child)
        .expect("the space was just created")
        .insert_or_replace(child_holder, ReservedLine::containing(MAIN));

    let cleared = rt
        .host_write(
            HostWriter::Lv2Effect,
            child,
            range(MAIN, 4),
            &[0xAB; 4],
            None,
        )
        .expect("the child region accepts the write");

    assert_eq!(cleared, 1);
    assert_eq!(
        rt.space_reservations(child)
            .expect("the space exists")
            .get(child_holder),
        None,
    );
    assert_eq!(
        rt.reservations().get(boot_holder),
        Some(ReservedLine::containing(MAIN)),
        "equal addresses in different spaces never alias, so space 0's table is untouched",
    );
    assert_eq!(
        rt.memory().read(range(MAIN, 4)).expect("mapped in space 0"),
        &[0u8; 4],
        "space 0's bytes are untouched",
    );
    assert_eq!(
        rt.space_memory(child)
            .expect("the space exists")
            .read(range(MAIN, 4))
            .expect("mapped in the child space"),
        &[0xAB; 4],
    );
    assert_eq!(
        host_writes(&rt),
        vec![TraceRecord::HostWrite {
            writer: HostWriter::Lv2Effect,
            space: child.raw(),
            addr: MAIN,
            len: 4,
            reservations_cleared: 1,
        }],
        "the record names the space the write landed in",
    );
}

#[test]
fn a_host_write_to_an_unmapped_range_is_refused_and_traces_nothing() {
    let mut rt = build();
    // Past the 4096-byte region at MAIN and short of the one at RSX.
    let err = rt.host_write(
        HostWriter::WakeContinuation,
        AddressSpaceId::BOOT,
        range(0x8000_0000, 4),
        &[0xAB; 4],
        None,
    );

    assert!(matches!(err, Err(MemError::Unmapped(_))));
    assert!(host_writes(&rt).is_empty());
}

#[test]
fn a_host_write_whose_payload_disagrees_with_its_range_is_refused() {
    let mut rt = build();
    rt.reservations_mut()
        .insert_or_replace(UnitId::new(1), ReservedLine::containing(MAIN));

    let err = rt.host_write(
        HostWriter::Lv2Effect,
        AddressSpaceId::BOOT,
        range(MAIN, 4),
        &[0xAB; 8],
        None,
    );

    assert!(matches!(err, Err(MemError::LengthMismatch)));
    assert_eq!(
        rt.reservations().get(UnitId::new(1)),
        Some(ReservedLine::containing(MAIN)),
        "a caller-side length bug must not invalidate a reservation",
    );
    assert!(host_writes(&rt).is_empty());
}

#[test]
fn the_fault_driven_mode_suppresses_host_write_records() {
    let mut rt = build();
    rt.set_mode(RuntimeMode::FaultDriven);

    rt.host_write(
        HostWriter::RsxMirror,
        AddressSpaceId::BOOT,
        range(MAIN, 4),
        &[0xAB; 4],
        None,
    )
    .expect("a writable region accepts the write");

    assert!(host_writes(&rt).is_empty());
    assert_eq!(
        rt.memory()
            .read(range(MAIN, 4))
            .expect("the region is readable"),
        &[0xAB; 4],
        "suppressing the record must not suppress the write",
    );
}

#[test]
fn a_continuation_payload_lands_in_the_units_own_space() {
    let mut rt = build();
    let child = AddressSpaceId::new(1);
    rt.create_address_space(child).expect("space 1 is fresh");
    rt.space_memory_mut(child)
        .expect("the space was just created")
        .install_region(MAIN, 4096, "child_main", PageSize::Page64K)
        .expect("an empty space has no region to overlap");
    let waiter = UnitId::new(1);
    rt.assign_unit_space(waiter, child)
        .expect("the space was just created");

    rt.commit_bytes_at(
        HostWriter::WakeContinuation,
        waiter,
        MAIN,
        &0xAABB_CCDDu32.to_be_bytes(),
    );

    assert_eq!(
        rt.space_memory(child)
            .expect("the space exists")
            .read(range(MAIN, 4))
            .expect("mapped in the child space"),
        &0xAABB_CCDDu32.to_be_bytes(),
        "the pointer came from the waiter's own syscall, so it resolves in its space",
    );
    assert_eq!(
        rt.memory().read(range(MAIN, 4)).expect("mapped in space 0"),
        &[0u8; 4],
        "boot bytes landing here means the payload resolved the wrong space",
    );
}

#[test]
fn a_continuation_payload_clears_every_other_holder_of_its_granule() {
    let mut rt = build();
    let waiter = UnitId::new(1);
    let other = UnitId::new(2);
    rt.reservations_mut()
        .insert_or_replace(waiter, ReservedLine::containing(MAIN));
    rt.reservations_mut()
        .insert_or_replace(other, ReservedLine::containing(MAIN));

    rt.commit_bytes_at(
        HostWriter::WakeContinuation,
        waiter,
        MAIN,
        &0xAABB_CCDDu32.to_be_bytes(),
    );

    assert_eq!(rt.reservations().get(other), None);
    assert_eq!(
        rt.reservations().get(waiter),
        Some(ReservedLine::containing(MAIN)),
        "the kernel writes on the parked caller's behalf, so it keeps its own line",
    );
}
