//! The record of fetches from code before its instruction-modification
//! sequence completed.

use std::collections::VecDeque;

use cellgov_effects::{Effect, WritePayload};
use cellgov_event::UnitId;
use cellgov_exec::{
    CodeEvent, ExecutionContext, ExecutionStepResult, ExecutionUnit, LocalDiagnostics, UnitStatus,
    YieldReason,
};
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_time::{Budget, GuestTicks, InstructionCost};
use cellgov_trace::{SEQUENCE_DCBST, SEQUENCE_ICBI, SEQUENCE_SYNC};

use super::{code_lines, is_patch_word, touches_code};
use crate::runtime::{AddressSpaceId, Runtime, UnsyncedCodeFetch};

/// The code word the programs rewrite.
pub(super) const X: u64 = 0x144;
/// A word on another code line.
const Y: u64 = 0x1C0;
const B_8: u32 = 0x4800_0008;
pub(super) const NOP: u32 = 0x6000_0000;

/// One scripted step: its events, and whether its batch faults.
#[derive(Clone, Default)]
pub(super) struct Step {
    pub(super) events: Vec<CodeEvent>,
    pub(super) fault: bool,
}

/// Runs one scripted step per batch. It emits each event's store as an
/// effect and hands the runtime the step's events.
#[derive(Clone)]
pub(super) struct Scripted {
    pub(super) id: UnitId,
    pub(super) steps: VecDeque<Step>,
    pub(super) drained: Vec<CodeEvent>,
}

impl ExecutionUnit for Scripted {
    type Snapshot = ();

    fn unit_id(&self) -> UnitId {
        self.id
    }

    fn status(&self) -> UnitStatus {
        if self.steps.is_empty() {
            UnitStatus::Finished
        } else {
            UnitStatus::Runnable
        }
    }

    fn run_until_yield(
        &mut self,
        _budget: Budget,
        _ctx: &ExecutionContext<'_>,
        effects: &mut Vec<Effect>,
    ) -> ExecutionStepResult {
        let step = self.steps.pop_front().unwrap_or_default();
        for event in &step.events {
            if let CodeEvent::Store {
                addr,
                word: Some(w),
                ..
            } = *event
            {
                effects.push(Effect::SharedWriteIntent {
                    range: ByteRange::new(GuestAddr::new(addr), 4).unwrap(),
                    bytes: WritePayload::new(w.to_be_bytes().to_vec()),
                    source: self.id,
                    source_time: GuestTicks::new(0),
                });
            }
        }
        self.drained = step.events;
        ExecutionStepResult {
            yield_reason: if step.fault {
                YieldReason::Fault
            } else {
                YieldReason::BudgetExhausted
            },
            consumed_cost: InstructionCost::new(1),
            local_diagnostics: LocalDiagnostics::empty(),
            fault: None,
            syscall_args: None,
        }
    }

    fn drain_code_events(&mut self) -> Vec<CodeEvent> {
        std::mem::take(&mut self.drained)
    }

    fn snapshot(&self) {}
}

pub(super) fn store(addr: u64, word: u32) -> CodeEvent {
    CodeEvent::Store {
        addr,
        len: 4,
        word: Some(word),
    }
}

pub(super) fn fetch(pc: u64) -> CodeEvent {
    CodeEvent::Fetch { pc }
}

pub(super) fn step(events: &[CodeEvent]) -> Step {
    Step {
        events: events.to_vec(),
        fault: false,
    }
}

/// The full sequence on the line of `addr`.
fn sequence(addr: u64) -> [CodeEvent; 4] {
    [
        CodeEvent::Dcbst { addr },
        CodeEvent::Sync,
        CodeEvent::Icbi { addr },
        CodeEvent::Isync,
    ]
}

/// Guest memory of 0x400 bytes with `[0x100, 0x200)` declared as code
/// and `X` holding `b +8`.
pub(super) fn memory() -> GuestMemory {
    let mut mem = GuestMemory::new(0x400);
    let range = ByteRange::new(GuestAddr::new(X), 4).unwrap();
    mem.apply_commit(range, &B_8.to_be_bytes()).unwrap();
    mem.declare_code(0x100, 0x100);
    mem
}

/// Runs one unit per script and returns the records and the unit ids.
/// The units take turns, one step at a time.
fn run(budget: u64, scripts: &[Vec<Step>]) -> (Vec<UnsyncedCodeFetch>, Vec<UnitId>) {
    let mut rt = Runtime::new(memory(), Budget::new(budget), 1000);
    let ids = scripts
        .iter()
        .map(|s| {
            let steps: VecDeque<Step> = s.iter().cloned().collect();
            rt.register_unit_with(|id| Scripted {
                id,
                steps,
                drained: Vec::new(),
            })
        })
        .collect();
    for _ in 0..64 {
        let Ok(mut s) = rt.step() else { break };
        rt.commit_step(&mut s).expect("the batch commits");
    }
    (rt.unsynced_code_fetches().to_vec(), ids)
}

fn run_one(steps: &[Step]) -> (Vec<UnsyncedCodeFetch>, UnitId) {
    let (records, ids) = run(1, &[steps.to_vec()]);
    (records, ids[0])
}

#[test]
fn a_fetch_after_a_store_with_no_sequence_is_recorded_once() {
    let (records, unit) = run_one(&[
        step(&[fetch(0x100), store(X, NOP)]),
        step(&[fetch(X), fetch(X + 4)]),
        step(&[fetch(X)]),
    ]);

    assert_eq!(
        records,
        [UnsyncedCodeFetch {
            unit,
            pc: X,
            space: AddressSpaceId::BOOT,
            line: 0x100,
            writer: unit,
            store_step: 1,
            sequence: 0,
            patch_class: true,
        }]
    );
}

#[test]
fn the_full_sequence_leaves_no_record() {
    let mut events = vec![store(X, NOP)];
    events.extend(sequence(X));
    events.push(fetch(X));
    let (records, _) = run_one(&[step(&events)]);
    assert_eq!(records, []);
}

#[test]
fn each_step_left_out_gives_a_record_naming_the_steps_that_ran() {
    let all = SEQUENCE_DCBST | SEQUENCE_SYNC | SEQUENCE_ICBI;
    let cases = [
        (0, all & !SEQUENCE_DCBST & !SEQUENCE_SYNC, "dcbst"),
        (1, all & !SEQUENCE_SYNC, "sync"),
        (2, SEQUENCE_DCBST | SEQUENCE_SYNC, "icbi"),
        (3, all, "isync"),
    ];
    for (left_out, sequence_bits, name) in cases {
        let mut events = vec![store(X, NOP)];
        events.extend(
            sequence(X)
                .iter()
                .enumerate()
                .filter(|&(i, _)| i != left_out)
                .map(|(_, e)| *e),
        );
        events.push(fetch(X));
        let (records, _) = run_one(&[step(&events)]);

        let [record] = records[..] else {
            panic!("without {name}: one record, got {records:?}");
        };
        assert_eq!(record.sequence, sequence_bits, "without {name}");
    }
}

#[test]
fn another_units_context_synchronization_after_the_writers_icbi_clears_the_line() {
    let writer = vec![step(&[
        store(X, NOP),
        CodeEvent::Dcbst { addr: X },
        CodeEvent::Sync,
        CodeEvent::Icbi { addr: X },
        CodeEvent::Sync,
    ])];
    for (sync, name) in [(CodeEvent::Isync, "isync"), (CodeEvent::ContextSync, "sc")] {
        let reader = vec![step(&[]), step(&[sync, fetch(X)])];
        let (records, _) = run(1, &[writer.clone(), reader]);
        assert_eq!(records, [], "the reader ran {name}");
    }

    let reader = vec![step(&[]), step(&[fetch(X)])];
    let (records, ids) = run(1, &[writer, reader]);
    let [record] = records[..] else {
        panic!("one record, got {records:?}");
    };
    assert_eq!((record.unit, record.writer), (ids[1], ids[0]));
    assert_eq!(
        record.sequence,
        SEQUENCE_DCBST | SEQUENCE_SYNC | SEQUENCE_ICBI
    );
}

#[test]
fn a_line_is_recorded_once_for_each_unit_that_fetches_it() {
    let writer = vec![step(&[store(X, NOP), fetch(X)])];
    let reader = vec![
        step(&[]),
        step(&[fetch(X), fetch(X + 4)]),
        step(&[fetch(X)]),
    ];
    let (records, ids) = run(1, &[writer, reader.clone(), reader]);

    let mut units: Vec<UnitId> = records.iter().map(|r| r.unit).collect();
    units.sort();
    assert_eq!(units, ids, "{records:?}");
    for record in &records {
        assert_eq!((record.writer, record.line), (ids[0], 0x100), "{record:?}");
    }
}

#[test]
fn the_writers_own_sc_does_not_end_its_sequence() {
    let mut events = vec![store(X, NOP)];
    events.extend(sequence(X)[..3].iter().copied());
    events.extend([CodeEvent::ContextSync, fetch(X)]);
    let (records, _) = run_one(&[step(&events)]);
    assert_eq!(records.len(), 1, "{records:?}");
}

#[test]
fn a_swap_out_of_the_patch_class_is_not_marked_patch_class() {
    let (records, _) = run_one(&[step(&[store(X, 0x3860_0001), fetch(X)])]);
    assert!(!records[0].patch_class, "{records:?}");

    let (records, _) = run_one(&[step(&[store(Y, NOP), fetch(Y)])]);
    assert!(
        !records[0].patch_class,
        "the old word is no branch: {records:?}"
    );

    let (records, _) = run_one(&[step(&[store(X, NOP), store(X, B_8), fetch(X)])]);
    assert!(records[0].patch_class, "branch, nop, branch: {records:?}");

    let (records, _) = run_one(&[step(&[store(X + 4, NOP), store(X, NOP), fetch(X)])]);
    assert!(
        !records[0].patch_class,
        "the first store's word held no patch-class word: {records:?}"
    );

    // The icbi starts a new mark, whose store replaces the `li` the
    // batch stored first.
    let (records, _) = run_one(&[step(&[
        store(X, 0x3860_0001),
        CodeEvent::Dcbst { addr: X },
        CodeEvent::Sync,
        CodeEvent::Icbi { addr: X },
        store(X, NOP),
        fetch(X),
    ])]);
    assert!(
        !records[0].patch_class,
        "li to nop is no patch-class swap: {records:?}"
    );
}

#[test]
fn a_new_store_reopens_a_reported_line() {
    let (records, _) = run_one(&[
        step(&[store(X, NOP), fetch(X)]),
        step(&[store(X, B_8), fetch(X)]),
    ]);
    assert_eq!(
        records.iter().map(|r| r.store_step).collect::<Vec<_>>(),
        [1, 2]
    );
}

#[test]
fn a_faulted_batch_replays_nothing() {
    let (records, _) = run_one(&[
        Step {
            events: vec![store(X, NOP), fetch(X)],
            fault: true,
        },
        step(&[fetch(X)]),
    ]);
    assert_eq!(records, []);
}

#[test]
fn a_fetch_outside_code_and_a_store_outside_code_record_nothing() {
    let (records, _) = run_one(&[step(&[store(0x204, NOP), fetch(0x204), fetch(X)])]);
    assert_eq!(records, []);
}

#[test]
fn a_restore_clears_the_marks_and_the_records() {
    let mut rt = Runtime::new(memory(), Budget::new(1), 1000);
    let steps: VecDeque<Step> = [
        step(&[]),
        step(&[store(X, NOP), fetch(X)]),
        step(&[fetch(X)]),
    ]
    .into_iter()
    .collect();
    rt.register_unit_with(|id| Scripted {
        id,
        steps,
        drained: Vec::new(),
    });
    let mut s = rt.step().unwrap();
    rt.commit_step(&mut s).unwrap();
    let snap = rt.snapshot();
    let mut s = rt.step().unwrap();
    rt.commit_step(&mut s).unwrap();
    assert_eq!(rt.unsynced_code_fetches().len(), 1);

    rt.restore_into(&snap);
    assert_eq!(rt.unsynced_code_fetches(), []);
}

#[test]
fn the_patch_class_is_the_i_form_branch_and_the_preferred_nop() {
    assert!(is_patch_word(B_8));
    assert!(is_patch_word(0x4BFF_FFF1), "bl backward");
    assert!(is_patch_word(NOP));
    assert!(!is_patch_word(0x6000_0001), "ori 0,0,1");
    assert!(!is_patch_word(0x4182_0008), "beq: a B-form branch");
}

#[test]
fn a_range_reaches_the_code_lines_it_overlaps() {
    let spans = [(0x100, 0x180), (0x200, 0x240)];
    assert!(touches_code(&spans, 0x17C, 0x180));
    assert!(!touches_code(&spans, 0x180, 0x200));
    assert!(touches_code(&spans, 0x1FF, 0x201));
    assert_eq!(code_lines(&spans, 0x17C, 8).collect::<Vec<_>>(), [0x100]);
    assert_eq!(code_lines(&spans, 0x1FC, 8).collect::<Vec<_>>(), [0x200]);
    assert_eq!(
        code_lines(&spans, 0xF8, 16).collect::<Vec<_>>(),
        [0x100],
        "a store from below into the first line"
    );
    assert_eq!(
        code_lines(&[(u64::MAX - 0x7F, u64::MAX)], u64::MAX - 3, 4).collect::<Vec<_>>(),
        [u64::MAX - 0x7F],
        "the last line of the address space"
    );
}
