//! The two per-step records a consumer reads off the runtime: the
//! tagged host writes and the LV2 effects that landed.

use cellgov_effects::{Effect, MailboxMessage, WritePayload};
use cellgov_exec::{ExecutionStepResult, LocalDiagnostics, YieldReason};
use cellgov_mem::{GuestAddr, GuestMemory, PageSize, Region, RegionAccess};
use cellgov_sync::MailboxId;
use cellgov_time::{Budget, GuestTicks, InstructionCost};
use cellgov_trace::TraceReader;

use super::*;

const MAIN: u64 = 0;
const RESERVED: u64 = 0xC000_0000;

/// Space 0 with a writable region at `MAIN` and a reserved region at
/// `RESERVED` that refuses every write.
fn build() -> Runtime {
    let memory = GuestMemory::from_regions(vec![
        Region::new(MAIN, 4096, "main", PageSize::Page64K),
        Region::with_access(
            RESERVED,
            256,
            "reserved",
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

fn write_intent(target: ByteRange, source: UnitId) -> Effect {
    Effect::shared_write(
        target,
        WritePayload::new(vec![0xAB; target.length() as usize]),
        source,
        GuestTicks::ZERO,
    )
}

fn traced_host_writes(rt: &Runtime) -> Vec<TraceRecord> {
    TraceReader::new(rt.trace().bytes())
        .map(|r| r.expect("the runtime's own stream decodes"))
        .filter(|r| matches!(r, TraceRecord::HostWrite { .. }))
        .collect()
}

/// A result the trivial fast path accepts: no effects, no fault, and a
/// yield reason that needs no arbitration.
fn trivial_result() -> ExecutionStepResult {
    ExecutionStepResult {
        yield_reason: YieldReason::BudgetExhausted,
        consumed_cost: InstructionCost::new(1),
        local_diagnostics: LocalDiagnostics::empty(),
        fault: None,
        syscall_args: None,
    }
}

#[test]
fn a_placement_before_the_first_commit_stands_in_the_published_host_writes() {
    let mut rt = build();

    rt.place_bytes(AddressSpaceId::BOOT, range(MAIN, 4), &[0xAB; 4])
        .expect("a writable region accepts the placement");

    assert_eq!(
        rt.last_host_writes().to_vec(),
        vec![(HostWriter::Placement, range(MAIN, 4))],
        "the list is emptied at commit entry, so a placement made before \
         the first commit is still in it",
    );
}

#[test]
fn the_trivial_fast_path_clears_the_records_a_placement_left_behind() {
    let mut rt = build();
    rt.set_mode(RuntimeMode::FaultDriven);
    rt.place_bytes(AddressSpaceId::BOOT, range(MAIN, 4), &[0xAB; 4])
        .expect("a writable region accepts the placement");

    rt.commit_step(&trivial_result(), &[])
        .expect("a trivial step commits");

    assert!(
        rt.last_host_writes().is_empty(),
        "the fast path returns before the body runs, so the clear has to \
         precede it or the next reader sees the previous step's writes",
    );
}

#[test]
fn a_refused_host_write_publishes_no_record() {
    let mut rt = build();

    rt.host_write(
        HostWriter::RsxMirror,
        AddressSpaceId::BOOT,
        range(RESERVED, 4),
        &[0xAB; 4],
        None,
    )
    .expect_err("a write into the reserved region cannot commit");

    assert!(rt.last_host_writes().is_empty());
}

#[test]
fn the_fault_driven_mode_publishes_the_record_it_does_not_trace() {
    let mut rt = build();
    rt.set_mode(RuntimeMode::FaultDriven);

    rt.host_write(
        HostWriter::DmaCompletion,
        AddressSpaceId::BOOT,
        range(MAIN, 8),
        &[0xAB; 8],
        None,
    )
    .expect("a writable region accepts the write");

    assert_eq!(
        rt.last_host_writes().to_vec(),
        vec![(HostWriter::DmaCompletion, range(MAIN, 8))],
        "the published record is mode independent; only the trace record \
         is gated on the mode",
    );
    assert!(traced_host_writes(&rt).is_empty());
}

#[test]
fn a_rolled_back_lv2_memory_subset_publishes_neither_record() {
    let mut rt = build();
    let caller = UnitId::new(0);

    // The subset commits all-or-none, and the second intent cannot
    // land.
    rt.apply_lv2_effects(
        &[
            write_intent(range(MAIN, 4), caller),
            write_intent(range(RESERVED, 4), caller),
        ],
        AddressSpaceId::BOOT,
    );

    assert_eq!(
        rt.lv2_host()
            .invariant_break_site_count("dispatch.lv2_effect_apply_failed"),
        1,
    );
    assert!(rt.last_lv2_effects().is_empty());
    assert!(rt.last_host_writes().is_empty());
    assert_eq!(
        rt.memory().read(range(MAIN, 4)).expect("mapped"),
        &[0u8; 4],
        "the rolled-back subset landed no bytes either",
    );
}

#[test]
fn an_applied_lv2_write_reaches_both_published_records() {
    let mut rt = build();
    let caller = UnitId::new(0);
    let effect = write_intent(range(MAIN, 4), caller);

    rt.apply_lv2_effects(std::slice::from_ref(&effect), AddressSpaceId::BOOT);

    assert_eq!(rt.last_lv2_effects().to_vec(), vec![effect]);
    assert_eq!(
        rt.last_host_writes().to_vec(),
        vec![(HostWriter::Lv2Effect, range(MAIN, 4))],
        "a handler's write is an LV2 effect and a host write both; the two \
         lists name the same bytes from either end",
    );
}

#[test]
fn an_lv2_mailbox_send_to_an_unregistered_mailbox_names_its_break() {
    let mut rt = build();
    let effect = Effect::MailboxSend {
        mailbox: MailboxId::new(7),
        message: MailboxMessage::new(0xAABB_CCDD),
        source: UnitId::new(0),
    };

    rt.apply_lv2_effects(std::slice::from_ref(&effect), AddressSpaceId::BOOT);

    assert_eq!(
        rt.lv2_host()
            .invariant_break_site_count("runtime.apply_lv2_effects_mailbox_send_unregistered"),
        1,
        "the message reaches no mailbox, so the drop has to be named",
    );
    assert!(rt.mailbox_registry().get(MailboxId::new(7)).is_none());
    assert_eq!(
        rt.last_lv2_effects().to_vec(),
        vec![effect],
        "the wake half of the send still ran, so the effect is published \
         even though the message reached no reader",
    );
}

#[test]
fn a_restore_clears_both_published_records() {
    let mut rt = build();
    let snap = rt.snapshot();
    rt.apply_lv2_effects(
        &[write_intent(range(MAIN, 4), UnitId::new(0))],
        AddressSpaceId::BOOT,
    );
    assert!(!rt.last_lv2_effects().is_empty());
    assert!(!rt.last_host_writes().is_empty());

    rt.restore_into(&snap);

    assert!(rt.last_lv2_effects().is_empty());
    assert!(rt.last_host_writes().is_empty());
}
