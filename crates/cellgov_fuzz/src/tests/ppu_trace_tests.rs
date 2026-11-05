use std::path::Path;

use cellgov_ppu::differential::rpcs3_capture::CapturedRecord;
use cellgov_ppu::differential::{
    execute_state, execute_state_in, ControlRegisters, PpuStateSnapshot, RPCS3_FPSCR_MODELLED,
};
use cellgov_ps3_abi::hw::ppu_status::CR0_EQ;
use cellgov_sync::ReservedLine;

use super::*;
use crate::ppu_documented::Set;

const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/ppu_reference");

/// The console vector the tests hold records against: `add`'s first.
const ADD_VECTOR: &str = "ppu_fixed_point_arith-add_p0";

/// An inexact `fadd`: the console end sets FPSCR bits beyond FPCC.
const FADD_VECTOR: &str = "ppu_float_edges-rn/fadd/rn0";

/// A trace that recorded VRSAVE, the FPSCR and the VSCR.
const RECORDED: ControlRegisters = ControlRegisters::Recorded;

/// `divw r3, r4, r5`.
const DIVW: u32 = 0x7c64_2bd6;

/// `mftb r3`.
const MFTB: u32 = 0x7c6c_42e6;

/// `stfs f1, 0(r4)`.
const STFS: u32 = 0xd024_0000;

/// `stwcx. r3, 0, r4`.
const STWCX: u32 = 0x7c60_212d;

/// The storage window a store record carries: the word `r4` names and
/// the word after it.
const WINDOW: u64 = 0x1000;

fn console() -> Vec<ConsoleVector> {
    console_vectors(Path::new(FIXTURES)).expect("the committed references load")
}

fn add_vector(console: &[ConsoleVector]) -> ConsoleVector {
    console
        .iter()
        .find(|v| v.file == "add.json" && v.name == ADD_VECTOR)
        .expect("add.json holds the vector")
        .clone()
}

/// A record of `raw` from `pre` whose RPCS3 end is `post`.
fn record(raw: u32, pre: PpuStateSnapshot, post: PpuStateSnapshot) -> CapturedRecord {
    CapturedRecord {
        pc: 0x1_0000,
        raw_instruction: raw,
        thread_id: 1,
        pre_state: pre,
        post_state: post,
        control_registers: ControlRegisters::Recorded,
        pre_reservation_rtime: 0,
        post_reservation_rtime: 0,
        mem_addr: 0,
        mem_pre: Vec::new(),
        mem_post: Vec::new(),
    }
}

/// A record of `raw` from `pre` whose RPCS3 end is CellGov's end with
/// `change` applied.
fn changed(
    raw: u32,
    pre: PpuStateSnapshot,
    change: impl Fn(&mut PpuStateSnapshot),
) -> CapturedRecord {
    let mut post = execute_state(raw, &pre).expect("CellGov runs the word");
    change(&mut post);
    record(raw, pre, post)
}

/// A record of the store `raw` from `pre` over the eight-byte window at
/// [`WINDOW`], whose RPCS3 end is CellGov's end with `change` applied.
fn stored(
    raw: u32,
    pre: PpuStateSnapshot,
    change: impl Fn(&mut PpuStateSnapshot, &mut [u8]),
) -> CapturedRecord {
    let mem_pre = vec![0u8; 8];
    let (mut post, mut mem_post) =
        execute_state_in(raw, &pre, WINDOW, mem_pre.clone()).expect("CellGov runs the store");
    change(&mut post, &mut mem_post);
    CapturedRecord {
        mem_addr: WINDOW,
        mem_pre,
        mem_post,
        ..record(raw, pre, post)
    }
}

fn rt(word: u32) -> usize {
    ((word >> 21) & 0x1f) as usize
}

fn only_class(report: &PpuTraceReport) -> &PpuTraceClass {
    assert_eq!(report.failures.len(), 1, "{:?}", report.failures);
    &report.failures[0].class
}

#[test]
fn a_record_both_sides_agree_on_passes_under_its_kind() {
    let console = console();
    let vector = add_vector(&console);
    let report = replay_trace(
        &[changed(vector.word, vector.start.clone(), |_| {})],
        &console,
        PpuTraceOptions::default(),
    );
    assert_eq!(report.records, 1);
    assert_eq!(report.control_registers, Some(ControlRegisters::Recorded));
    assert_eq!(
        report.kinds,
        vec![PpuTraceKindRow {
            kind: "Ordinary(Add)".to_owned(),
            seen: 1,
            passed: 1,
            failed: 0,
            skipped: 0,
        }]
    );
    assert!(report.failures.is_empty());
    assert!(!report.unreached.contains(&"Ordinary(Add)".to_owned()));
    assert!(report.unreached.contains(&"Ordinary(Subf)".to_owned()));
}

#[test]
fn an_rpcs3_end_the_console_contradicts_is_an_rpcs3_divergence() {
    let console = console();
    let vector = add_vector(&console);
    let target = rt(vector.word);
    let wrong = changed(vector.word, vector.start.clone(), |post| {
        post.gpr[target] ^= 1
    });
    let report = replay_trace(
        std::slice::from_ref(&wrong),
        &console,
        PpuTraceOptions::default(),
    );
    assert_eq!(
        only_class(&report),
        &PpuTraceClass::Rpcs3Divergence {
            vector: format!("add.json {ADD_VECTOR}"),
        }
    );
    assert_eq!(report.kinds[0].failed, 1);
    let alone = replay_trace(&[wrong], &[], PpuTraceOptions::default());
    assert_eq!(only_class(&alone), &PpuTraceClass::Unclassified);
}

#[test]
fn an_rpcs3_end_the_console_supports_is_a_cellgov_defect() {
    let mut vector = add_vector(&console());
    let target = rt(vector.word);
    let mut rpcs3 = execute_state(vector.word, &vector.start).expect("CellGov runs the word");
    rpcs3.gpr[target] ^= 1;
    vector.expected.state.gpr = crate::reference::ReferenceField::Value {
        value: rpcs3.gpr.to_vec(),
    };
    let report = replay_trace(
        &[record(vector.word, vector.start.clone(), rpcs3)],
        std::slice::from_ref(&vector),
        PpuTraceOptions::default(),
    );
    assert_eq!(
        only_class(&report),
        &PpuTraceClass::CellGovDefect {
            vector: format!("add.json {ADD_VECTOR}"),
        }
    );
    assert!(only_class(&report).needs_triage());
}

#[test]
fn an_fpscr_bit_rpcs3_does_not_model_does_not_hide_a_cellgov_defect() {
    let mut vector = console()
        .into_iter()
        .find(|v| v.file == "fadd.json" && v.name == FADD_VECTOR)
        .expect("fadd.json holds the vector");
    let target = rt(vector.word);
    let cellgov = execute_state(vector.word, &vector.start).expect("CellGov runs the word");
    let mut rpcs3 = cellgov.clone();
    rpcs3.fpr[target] ^= 1;
    // RPCS3 sets FPCC alone; every other FPSCR bit keeps its start value.
    rpcs3.fpscr =
        (vector.start.fpscr & !RPCS3_FPSCR_MODELLED) | (cellgov.fpscr & RPCS3_FPSCR_MODELLED);
    let crate::reference::ReferenceField::Value {
        value: console_fpscr,
    } = &vector.expected.state.fpscr
    else {
        panic!("the vector reports the FPSCR");
    };
    assert_ne!(
        *console_fpscr, rpcs3.fpscr,
        "the console sets an unmodelled bit"
    );
    vector.expected.state.fpr = crate::reference::ReferenceField::Value {
        value: rpcs3.fpr.to_vec(),
    };
    let report = replay_trace(
        &[record(vector.word, vector.start.clone(), rpcs3)],
        std::slice::from_ref(&vector),
        PpuTraceOptions::default(),
    );
    assert_eq!(
        only_class(&report),
        &PpuTraceClass::CellGovDefect {
            vector: format!("fadd.json {FADD_VECTOR}"),
        }
    );
}

fn fadd_vector() -> ConsoleVector {
    console()
        .into_iter()
        .find(|v| v.file == "fadd.json" && v.name == FADD_VECTOR)
        .expect("fadd.json holds the vector")
}

/// The FPSCR the console's end reports.
fn console_fpscr(vector: &ConsoleVector) -> u32 {
    let crate::reference::ReferenceField::Value { value } = &vector.expected.state.fpscr else {
        panic!("the vector reports the FPSCR");
    };
    *value
}

#[test]
fn a_cellgov_fpscr_bit_outside_fpcc_still_counts_against_cellgov() {
    let vector = fadd_vector();
    let target = rt(vector.word);
    let mut cellgov = execute_state(vector.word, &vector.start).expect("CellGov runs the word");
    let mut traced = cellgov.clone();
    traced.fpr[target] ^= 1;
    // The trace's end holds FPCC alone; its other FPSCR bits keep their start values.
    traced.fpscr =
        (vector.start.fpscr & !RPCS3_FPSCR_MODELLED) | (cellgov.fpscr & RPCS3_FPSCR_MODELLED);
    assert_eq!(
        vector.decide((&traced, &[]), (&cellgov, &[]), 0, RECORDED),
        Some(ConsoleSide::CellGov),
        "CellGov's end agrees with the console at every bit"
    );
    let unmodelled = console_fpscr(&vector) ^ traced.fpscr;
    assert_ne!(unmodelled, 0, "the console sets a bit outside FPCC");
    cellgov.fpscr ^= unmodelled & unmodelled.wrapping_neg();
    assert_eq!(
        vector.decide((&traced, &[]), (&cellgov, &[]), 0, RECORDED),
        None
    );
}

#[test]
fn a_trace_that_did_not_record_the_control_registers_is_judged_on_none_of_them() {
    let vector = fadd_vector();
    let target = rt(vector.word);
    let right = execute_state(vector.word, &vector.start).expect("CellGov runs the word");
    let mut traced = right.clone();
    traced.fpscr = console_fpscr(&vector) ^ RPCS3_FPSCR_MODELLED;
    let mut wrong = right;
    wrong.fpr[target] ^= 1;
    assert_eq!(
        vector.decide(
            (&traced, &[]),
            (&wrong, &[]),
            0,
            ControlRegisters::NotRecorded
        ),
        Some(ConsoleSide::Rpcs3)
    );
    assert_eq!(
        vector.decide((&traced, &[]), (&wrong, &[]), 0, RECORDED),
        None,
        "a recorded FPCC the console contradicts is judged"
    );
}

#[test]
fn the_console_names_rpcs3_when_cellgov_alone_differs_from_it() {
    let vector = add_vector(&console());
    let target = rt(vector.word);
    let right = execute_state(vector.word, &vector.start).expect("CellGov runs the word");
    let mut wrong = right.clone();
    wrong.gpr[target] ^= 1;
    assert_eq!(
        vector.decide((&right, &[]), (&wrong, &[]), 0, RECORDED),
        Some(ConsoleSide::Rpcs3)
    );
    assert_eq!(
        vector.decide((&wrong, &[]), (&right, &[]), 0, RECORDED),
        Some(ConsoleSide::CellGov)
    );
    assert_eq!(
        vector.decide((&right, &[]), (&right, &[]), 0, RECORDED),
        None
    );
}

#[test]
fn a_register_the_vector_leaves_free_does_not_turn_an_rpcs3_divergence_into_a_defect() {
    let console = console();
    let vector = add_vector(&console);
    let target = rt(vector.word);
    let mut pre = vector.start.clone();
    pre.gpr[20] = 0x55;
    let wrong = changed(vector.word, pre, |post| post.gpr[target] ^= 1);
    assert!(vector.matches(&wrong), "a register no field names is free");
    let report = replay_trace(&[wrong], &console, PpuTraceOptions::default());
    assert_eq!(
        only_class(&report),
        &PpuTraceClass::Rpcs3Divergence {
            vector: format!("add.json {ADD_VECTOR}"),
        }
    );
}

#[test]
fn the_console_decides_nothing_when_it_agrees_with_neither_side() {
    let vector = add_vector(&console());
    let target = rt(vector.word);
    let right = execute_state(vector.word, &vector.start).expect("CellGov runs the word");
    let mut one = right.clone();
    one.gpr[target] ^= 1;
    let mut two = right;
    two.gpr[target] ^= 2;
    assert_eq!(vector.decide((&one, &[]), (&two, &[]), 0, RECORDED), None);
}

#[test]
fn a_cellgov_write_the_console_does_not_judge_blocks_an_rpcs3_divergence() {
    let vector = add_vector(&console());
    let target = rt(vector.word);
    let right = execute_state(vector.word, &vector.start).expect("CellGov runs the word");
    let mut rpcs3 = right.clone();
    rpcs3.gpr[target] ^= 1;
    let mut cellgov = right;
    cellgov.gpr[20] ^= 1;
    assert_eq!(
        vector.decide((&rpcs3, &[]), (&cellgov, &[]), 0, RECORDED),
        None
    );
}

#[test]
fn a_vector_matches_only_a_record_with_its_word_and_inputs() {
    let vector = add_vector(&console());
    let named = ((vector.word >> 16) & 0x1f) as usize;
    let base = record(vector.word, vector.start.clone(), vector.start.clone());
    assert!(vector.matches(&base));
    let mut unrelated = base.clone();
    unrelated.pre_state.gpr[20] = 0x55;
    assert!(
        vector.matches(&unrelated),
        "a register no field names is free"
    );
    let mut operand = base.clone();
    operand.pre_state.gpr[named] ^= 1;
    assert!(!vector.matches(&operand));
    let mut other_bank = base.clone();
    other_bank.pre_state.fpr[named] = 1;
    assert!(
        !vector.matches(&other_bank),
        "a named index is held in every bank"
    );
    let mut xer = base.clone();
    xer.pre_state.xer = 1 << 31;
    assert!(!vector.matches(&xer));
    let mut word = base;
    word.raw_instruction ^= 1 << 10;
    assert!(!vector.matches(&word));
}

#[test]
fn a_divide_by_zero_differing_only_in_rt_is_undefined() {
    let mut pre = PpuStateSnapshot::zero();
    pre.gpr[4] = 7;
    let open = changed(DIVW, pre.clone(), |post| post.gpr[3] ^= 0xdead_beef);
    let report = replay_trace(&[open], &[], PpuTraceOptions::default());
    assert_eq!(
        only_class(&report),
        &PpuTraceClass::Undefined {
            sets: vec![Set::MuldivUndef],
        }
    );
    let closed = changed(DIVW, pre.clone(), |post| {
        post.gpr[3] ^= 1;
        post.cr ^= 0x8000_0000;
    });
    let report = replay_trace(&[closed], &[], PpuTraceOptions::default());
    assert_eq!(only_class(&report), &PpuTraceClass::Unclassified);
    pre.gpr[5] = 2;
    let defined = changed(DIVW, pre, |post| post.gpr[3] ^= 1);
    let report = replay_trace(&[defined], &[], PpuTraceOptions::default());
    assert_eq!(only_class(&report), &PpuTraceClass::Unclassified);
}

#[test]
fn a_time_base_read_is_context_dependent_or_skipped() {
    let wrong = changed(MFTB, PpuStateSnapshot::zero(), |post| post.gpr[3] = 1234);
    let report = replay_trace(
        std::slice::from_ref(&wrong),
        &[],
        PpuTraceOptions::default(),
    );
    assert_eq!(only_class(&report), &PpuTraceClass::ContextDependent);
    let skipped = replay_trace(
        &[wrong],
        &[],
        PpuTraceOptions {
            skip_context_dependent: true,
        },
    );
    assert!(skipped.failures.is_empty());
    assert_eq!((skipped.kinds[0].seen, skipped.kinds[0].skipped), (1, 1));
}

#[test]
fn a_storage_difference_is_undefined_only_inside_the_open_bytes() {
    let mut pre = PpuStateSnapshot::zero();
    pre.fpr[1] = 1;
    pre.gpr[4] = WINDOW;
    let inside = stored(STFS, pre.clone(), |_, mem| mem[0] ^= 1);
    let report = replay_trace(&[inside], &[], PpuTraceOptions::default());
    assert_eq!(
        only_class(&report),
        &PpuTraceClass::Undefined {
            sets: vec![Set::StfsBelowDenormal],
        }
    );
    let outside = stored(STFS, pre, |_, mem| mem[4] ^= 1);
    let report = replay_trace(&[outside], &[], PpuTraceOptions::default());
    assert_eq!(only_class(&report), &PpuTraceClass::Unclassified);
}

#[test]
fn an_open_state_difference_does_not_hide_a_storage_difference_outside_the_open_bytes() {
    let mut pre = PpuStateSnapshot::zero();
    pre.gpr[4] = WINDOW;
    pre.reservation = Some(ReservedLine::containing(WINDOW + 0x1000));
    let inside = stored(STWCX, pre.clone(), |post, mem| {
        post.cr ^= CR0_EQ;
        mem[0] ^= 1;
    });
    let report = replay_trace(&[inside], &[], PpuTraceOptions::default());
    assert_eq!(
        only_class(&report),
        &PpuTraceClass::Undefined {
            sets: vec![Set::StwcxOtherAddress],
        }
    );
    let outside = stored(STWCX, pre, |post, mem| {
        post.cr ^= CR0_EQ;
        mem[4] ^= 1;
    });
    let report = replay_trace(&[outside], &[], PpuTraceOptions::default());
    assert_eq!(only_class(&report), &PpuTraceClass::ContextDependent);
}

#[test]
fn a_failure_names_each_field_that_differs() {
    let mut pre = PpuStateSnapshot::zero();
    pre.gpr[5] = 2;
    let wrong = changed(DIVW, pre, |post| {
        post.gpr[3] ^= 1;
        post.cr ^= 1;
    });
    let report = replay_trace(&[wrong], &[], PpuTraceOptions::default());
    assert_eq!(
        differing_fields(&report.failures[0].outcome),
        vec!["gpr3".to_owned(), "cr".to_owned()]
    );
}

#[test]
fn the_committed_references_give_single_word_console_vectors() {
    let console = console();
    assert!(console.len() > 1000, "{} vectors", console.len());
    assert!(console
        .iter()
        .any(|v| v.file == "add.json" && v.name == ADD_VECTOR));
    assert!(
        !console
            .iter()
            .any(|v| v.file == "addi.json" && v.name == "li-r3-7"),
        "a documented vector is not a console vector"
    );
    assert!(
        !console
            .iter()
            .any(|v| v.file == "vxor.json" && v.name == "ppu_vmx_int_edges-logic_xor"),
        "a console vector that runs two words is not a single-word vector"
    );
    assert!(console
        .iter()
        .any(|v| v.file == "vxor.json" && v.name == "ppu_vmx_permute-vxor/mix"));
}

#[test]
fn a_reference_that_does_not_parse_is_refused_by_name() {
    let dir = cellgov_testkit::scratch::scratch_labeled("ppu_trace_refs_refused");
    std::fs::write(dir.join("broken.json"), "{").expect("write");
    let refused = console_vectors(&dir);
    assert!(matches!(
        refused,
        Err(PpuTraceReferenceError::Parse { ref file, .. }) if file == "broken.json"
    ));
}
