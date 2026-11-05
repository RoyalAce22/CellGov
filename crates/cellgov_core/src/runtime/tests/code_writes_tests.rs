//! The record of committed guest stores into declared code.

use cellgov_effects::{Effect, WritePayload};
use cellgov_event::UnitId;
use cellgov_exec::{
    ExecutionContext, ExecutionStepResult, ExecutionUnit, LocalDiagnostics, UnitStatus, YieldReason,
};
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_time::{Budget, GuestTicks, InstructionCost};

use super::{first_code_word, CODE_WRITE_DETAIL_LIMIT};
use crate::runtime::{AddressSpaceId, CodeWrite, Runtime};

/// Always runnable; emits nothing, so each test supplies the batch.
#[derive(Clone)]
struct Idle {
    id: UnitId,
}

impl ExecutionUnit for Idle {
    type Snapshot = ();

    fn unit_id(&self) -> UnitId {
        self.id
    }

    fn status(&self) -> UnitStatus {
        UnitStatus::Runnable
    }

    fn run_until_yield(
        &mut self,
        budget: Budget,
        _ctx: &ExecutionContext<'_>,
        _effects: &mut Vec<Effect>,
    ) -> ExecutionStepResult {
        result(YieldReason::BudgetExhausted, budget.raw())
    }

    fn snapshot(&self) {}
}

fn result(yield_reason: YieldReason, cost: u64) -> ExecutionStepResult {
    ExecutionStepResult {
        yield_reason,
        consumed_cost: InstructionCost::new(cost),
        local_diagnostics: LocalDiagnostics::empty(),
        fault: None,
        syscall_args: None,
    }
}

/// A runtime over 0x400 bytes holding `0x11111111` in every word, with
/// `[0x100, 0x180)` declared as code, and one step taken.
fn runtime() -> (Runtime, UnitId) {
    let mut mem = GuestMemory::new(0x400);
    let range = ByteRange::new(GuestAddr::new(0), 0x400).unwrap();
    mem.apply_commit(range, &[0x11; 0x400]).unwrap();
    mem.declare_code(0x100, 0x80);
    let mut rt = Runtime::new(mem, Budget::new(1), 1000);
    let unit = rt.register_unit_with(|id| Idle { id });
    rt.step().expect("the unit is runnable");
    (rt, unit)
}

fn store(unit: UnitId, addr: u64, bytes: &[u8]) -> Effect {
    Effect::SharedWriteIntent {
        range: ByteRange::new(GuestAddr::new(addr), bytes.len() as u64).unwrap(),
        bytes: WritePayload::new(bytes.to_vec()),
        source: unit,
        source_time: GuestTicks::new(0),
    }
}

fn commit(rt: &mut Runtime, yield_reason: YieldReason, effects: &[Effect]) {
    rt.commit_unchecked(&result(yield_reason, 1), effects)
        .expect("the batch commits");
}

#[test]
fn a_store_into_code_records_its_writer_step_and_word() {
    let (mut rt, unit) = runtime();
    commit(
        &mut rt,
        YieldReason::BudgetExhausted,
        &[store(unit, 0x144, &[0x60, 0, 0, 0])],
    );

    let w = rt.code_writes();
    assert_eq!(w.stores(), 1);
    assert_eq!(
        w.detail(),
        [CodeWrite {
            unit,
            space: AddressSpaceId::BOOT,
            step: 1,
            word: 0x144,
            before: 0x1111_1111,
            after: 0x6000_0000,
        }]
    );
    assert_eq!(
        w.lines().collect::<Vec<_>>(),
        [(AddressSpaceId::BOOT, 0x100, 1)]
    );
}

#[test]
fn a_store_outside_code_is_not_recorded() {
    let (mut rt, unit) = runtime();
    commit(
        &mut rt,
        YieldReason::BudgetExhausted,
        &[store(unit, 0xFC, &[0; 4]), store(unit, 0x180, &[0; 4])],
    );

    assert_eq!(rt.code_writes().stores(), 0);
    assert!(rt.code_writes().detail().is_empty());
}

#[test]
fn a_conditional_store_into_code_is_recorded() {
    let (mut rt, unit) = runtime();
    let cond = Effect::ConditionalStore {
        range: ByteRange::new(GuestAddr::new(0x17C), 4).unwrap(),
        bytes: WritePayload::new(vec![0x48, 0, 0, 0x10]),
        source: unit,
        source_time: GuestTicks::new(0),
    };
    commit(&mut rt, YieldReason::BudgetExhausted, &[cond]);

    let w = rt.code_writes();
    assert_eq!(w.stores(), 1);
    assert_eq!(w.detail()[0].word, 0x17C);
    assert_eq!(w.detail()[0].after, 0x4800_0010);
}

#[test]
fn a_faulted_batch_records_nothing() {
    let (mut rt, unit) = runtime();
    commit(
        &mut rt,
        YieldReason::Fault,
        &[store(unit, 0x144, &[0x60, 0, 0, 0])],
    );

    assert_eq!(rt.code_writes().stores(), 0);
}

#[test]
fn stores_past_the_detail_limit_still_count() {
    let (mut rt, unit) = runtime();
    let n = CODE_WRITE_DETAIL_LIMIT + 3;
    let effects: Vec<Effect> = (0..n)
        .map(|i| store(unit, 0x100 + 4 * (i as u64 % 32), &[0; 4]))
        .collect();
    commit(&mut rt, YieldReason::BudgetExhausted, &effects);

    let w = rt.code_writes();
    assert_eq!(w.stores(), n as u64);
    assert_eq!(w.detail().len(), CODE_WRITE_DETAIL_LIMIT);
    assert_eq!(
        w.lines().collect::<Vec<_>>(),
        [(AddressSpaceId::BOOT, 0x100, n as u64)]
    );
}

#[test]
fn a_restore_keeps_the_record() {
    let (mut rt, unit) = runtime();
    let snap = rt.snapshot();
    commit(
        &mut rt,
        YieldReason::BudgetExhausted,
        &[store(unit, 0x144, &[0; 4])],
    );
    rt.restore_into(&snap);

    assert_eq!(rt.code_writes().stores(), 1);
}

#[test]
fn the_first_code_word_is_the_lowest_word_inside_a_span() {
    let spans = [(0x100, 0x180), (0x200, 0x240)];
    let at =
        |start, len| first_code_word(&spans, ByteRange::new(GuestAddr::new(start), len).unwrap());

    assert_eq!(at(0xF0, 0x20), Some(0x100), "a store running into a span");
    assert_eq!(
        at(0x17E, 4),
        Some(0x17C),
        "a misaligned store at a span's end"
    );
    assert_eq!(at(0x180, 0x81), Some(0x200), "a store crossing the gap");
    assert_eq!(at(0x180, 0x80), None, "a store ending at the next span");
    assert_eq!(at(0x240, 4), None, "a store past the last span");
}
