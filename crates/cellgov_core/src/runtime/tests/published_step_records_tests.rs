//! The two per-step records a consumer reads off the runtime: the
//! tagged host writes and the LV2 effects that landed.

use std::cell::Cell;

use cellgov_dma::{DmaDirection, DmaRequest};
use cellgov_effects::{Effect, MailboxMessage, WritePayload};
use cellgov_exec::{
    ExecutionContext, ExecutionStepResult, ExecutionUnit, LocalDiagnostics, UnitStatus, YieldReason,
};
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
fn a_placement_before_the_first_step_stands_in_the_published_host_writes() {
    let mut rt = build();

    rt.place_bytes(AddressSpaceId::BOOT, range(MAIN, 4), &[0xAB; 4])
        .expect("a writable region accepts the placement");

    assert_eq!(
        rt.last_host_writes().to_vec(),
        vec![(HostWriter::Placement, range(MAIN, 4))],
        "the list is emptied at step entry, so a placement made before \
         the first step is still in it",
    );
}

#[test]
fn a_commit_keeps_the_records_and_the_next_step_clears_them() {
    let mut rt = build();
    rt.set_mode(RuntimeMode::FaultDriven);
    rt.register_unit_with(|id| DmaWaiter {
        id,
        steps: Cell::new(0),
    });
    rt.place_bytes(AddressSpaceId::BOOT, range(MAIN, 4), &[0xAB; 4])
        .expect("a writable region accepts the placement");

    rt.commit_step(&trivial_result(), &[])
        .expect("a trivial step commits");
    assert_eq!(
        rt.last_host_writes().to_vec(),
        vec![(HostWriter::Placement, range(MAIN, 4))],
        "a commit clears nothing, including on the fast path: the time warp \
         runs ahead of it and what the warp wrote has to survive it",
    );

    rt.step().expect("the waiter is runnable");
    assert!(
        rt.last_host_writes().is_empty(),
        "the step is what opens the record",
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

const DMA_SRC: u64 = 0x100;
const DMA_DST: u64 = 0x200;

/// Enqueues one transfer on its first step and parks on it.
///
/// The test registers nothing else, so the transfer can only land in
/// the time warp the next [`Runtime::step`] takes.
#[derive(Clone)]
struct DmaWaiter {
    id: UnitId,
    steps: Cell<u64>,
}

impl ExecutionUnit for DmaWaiter {
    type Snapshot = u64;

    fn unit_id(&self) -> UnitId {
        self.id
    }

    fn status(&self) -> UnitStatus {
        if self.steps.get() >= 2 {
            UnitStatus::Finished
        } else {
            UnitStatus::Runnable
        }
    }

    fn run_until_yield(
        &mut self,
        budget: Budget,
        _ctx: &ExecutionContext<'_>,
        effects: &mut Vec<Effect>,
    ) -> ExecutionStepResult {
        let n = self.steps.get() + 1;
        self.steps.set(n);
        let yield_reason = if n == 1 {
            let request = DmaRequest::new(
                DmaDirection::Put,
                range(DMA_SRC, 8),
                range(DMA_DST, 8),
                self.id,
            )
            .expect("a put between two mapped, disjoint ranges");
            effects.push(Effect::DmaEnqueue {
                request,
                payload: None,
            });
            YieldReason::DmaWait
        } else {
            YieldReason::Finished
        };
        ExecutionStepResult {
            yield_reason,
            consumed_cost: InstructionCost::new(budget.raw()),
            local_diagnostics: LocalDiagnostics::empty(),
            fault: None,
            syscall_args: None,
        }
    }

    fn snapshot(&self) -> u64 {
        self.steps.get()
    }
}

#[test]
fn a_transfer_the_time_warp_fires_reaches_the_published_host_writes() {
    let mut rt = build();
    let waiter = rt.register_unit_with(|id| DmaWaiter {
        id,
        steps: Cell::new(0),
    });

    let step = rt.step().expect("the waiter is runnable");
    rt.commit_step(&step.result, &step.effects)
        .expect("the enqueue commits");
    assert!(
        !rt.dma_queue().is_empty(),
        "the premise: the transfer is still in flight, so only a warp lands it",
    );
    assert_eq!(
        rt.registry().effective_status(waiter),
        Some(UnitStatus::Blocked),
        "and nothing is runnable, so the next step has to warp",
    );

    let step = rt.step().expect("the warp wakes the issuer");
    rt.commit_step(&step.result, &step.effects)
        .expect("the woken step commits");

    assert!(
        rt.last_host_writes()
            .contains(&(HostWriter::DmaCompletion, range(DMA_DST, 8))),
        "the warp landed the transfer before it picked a step, and the commit \
         that followed must not clear what it wrote: {:?}",
        rt.last_host_writes(),
    );
}

const FLAG_ID: u32 = 1;
const RESULT_PTR: u32 = 0x300;

/// Waits on an event flag whose bits never arrive, with a finite
/// timeout, then finishes.
///
/// The test registers nothing else, so only the time warp reaches the
/// deadline. The expiry writes the observed bits back through the
/// result pointer from inside the warp.
#[derive(Clone)]
struct FlagWaiter {
    id: UnitId,
    steps: Cell<u64>,
}

impl ExecutionUnit for FlagWaiter {
    type Snapshot = u64;

    fn unit_id(&self) -> UnitId {
        self.id
    }

    fn status(&self) -> UnitStatus {
        if self.steps.get() >= 2 {
            UnitStatus::Finished
        } else {
            UnitStatus::Runnable
        }
    }

    fn run_until_yield(
        &mut self,
        budget: Budget,
        _ctx: &ExecutionContext<'_>,
        _effects: &mut Vec<Effect>,
    ) -> ExecutionStepResult {
        let n = self.steps.get() + 1;
        self.steps.set(n);
        let (yield_reason, syscall_args) = if n == 1 {
            let mut args = [0u64; 9];
            args[0] = cellgov_ps3_abi::lv2::syscall::EVENT_FLAG_WAIT;
            args[1] = u64::from(FLAG_ID);
            args[2] = 0b10;
            args[3] = 0x01;
            args[4] = u64::from(RESULT_PTR);
            args[5] = 1_000;
            (YieldReason::Syscall, Some(args))
        } else {
            (YieldReason::Finished, None)
        };
        ExecutionStepResult {
            yield_reason,
            consumed_cost: InstructionCost::new(budget.raw()),
            local_diagnostics: LocalDiagnostics::with_pc(0x1000),
            fault: None,
            syscall_args,
        }
    }

    fn snapshot(&self) -> u64 {
        self.steps.get()
    }
}

#[test]
fn an_expiry_the_time_warp_fires_reaches_the_published_records() {
    let mut rt = build();
    let waiter = rt.register_unit_with(|id| FlagWaiter {
        id,
        steps: Cell::new(0),
    });
    rt.lv2_host_mut().seed_primary_ppu_thread(
        waiter,
        cellgov_lv2::PpuThreadAttrs {
            entry: 0,
            arg: 0,
            stack_base: 0,
            stack_size: 0,
            priority: 0,
            tls_base: 0,
        },
    );
    rt.lv2_host_mut()
        .event_flags_mut()
        .create_with_id(FLAG_ID, 0)
        .expect("a fresh event-flag id");

    let step = rt.step().expect("the waiter is runnable");
    rt.commit_step(&step.result, &step.effects)
        .expect("the wait commits");
    assert_eq!(
        rt.registry().effective_status(waiter),
        Some(UnitStatus::Blocked),
        "the premise: the wait parked, so only the warp reaches its deadline",
    );

    let step = rt.step().expect("the warp expires the wait");
    rt.commit_step(&step.result, &step.effects)
        .expect("the woken step commits");

    assert!(
        rt.last_host_writes()
            .contains(&(HostWriter::Lv2Effect, range(u64::from(RESULT_PTR), 8))),
        "the expiry wrote the observed bits through the waiter's result \
         pointer inside the warp, and the commit after it must not clear \
         what the warp wrote: {:?}",
        rt.last_host_writes(),
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
