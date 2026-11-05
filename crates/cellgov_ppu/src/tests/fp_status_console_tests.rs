//! The arithmetic arms' and the estimates' FPSCR status held to two
//! captures from the reference console, `ppu_fp_scalar_rules` and
//! `ppu_fp_status_writeback`. Each case runs the probe's one instruction
//! on the probe's operands, from the probe's FPSCR. The test compares FRT
//! and the FPSCR with the words the console stored.
//!
//! frsp writes no status. A constant below names its cases, and
//! its test asserts both the console's value and CellGov's different
//! value.

use crate::fp::{execute_fp59, execute_fp63};
use crate::instruction::ops::{Fp59Op, Fp63Op};
use crate::state::PpuState;

/// One case of a probe capture.
///
/// `op` is the operation number that the probe's `ppu/main.c` gives.
/// `word1` is the FPSCR that the instruction starts from.
pub(super) struct Case {
    pub(super) op: u32,
    pub(super) word1: u32,
    pub(super) a: u64,
    pub(super) b: u64,
    pub(super) c: u64,
    pub(super) frt: u64,
    pub(super) fpscr: u32,
}

/// The cases of `probe`'s capture: 3 header words, then 11 words for each case.
pub(super) fn cases(probe: &str, count: usize) -> Vec<Case> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/micro")
        .join(probe)
        .join("ps3/cech20-cex-493/cgov_frame.bin");
    let frame = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    assert_eq!(&frame[0..4], b"CGOV", "frame magic");
    let w: Vec<u32> = frame[8..]
        .chunks_exact(4)
        .map(|w| u32::from_be_bytes(w.try_into().expect("4 bytes")))
        .collect();
    assert_eq!((w[0], w[1]), (0, 0), "{probe} stopped at a step");
    assert_eq!(w[2] as usize, count, "{probe} case count");
    assert_eq!(w.len(), 3 + 11 * count, "{probe} payload length");
    let pair = |at: usize| u64::from(w[at]) << 32 | u64::from(w[at + 1]);
    (0..count)
        .map(|k| {
            let at = 3 + 11 * k;
            Case {
                op: w[at],
                word1: w[at + 1],
                a: pair(at + 2),
                b: pair(at + 4),
                c: pair(at + 6),
                frt: pair(at + 8),
                fpscr: w[at + 10],
            }
        })
        .collect()
}

/// FRT and the FPSCR after `op` on FRA = f1, FRB = f2, FRC = f3 from
/// FPSCR `pre`, with FRT = f0.
pub(super) fn run(op: Fp, pre: u32, a: u64, b: u64, c: u64) -> (u64, u32) {
    let mut s = PpuState::new();
    s.set_fpscr(pre);
    s.set_fpr(1, a);
    s.set_fpr(2, b);
    s.set_fpr(3, c);
    match op {
        Fp::D(op) => execute_fp63(&mut s, op, 0, 1, 2, 3),
        Fp::S(op) => execute_fp59(&mut s, op, 0, 1, 2, 3),
    };
    (s.fpr[0], s.fpscr())
}

/// A primary-63 or a primary-59 operation.
#[derive(Debug, Clone, Copy)]
pub(super) enum Fp {
    D(Fp63Op),
    S(Fp59Op),
}

/// The `ppu_fp_scalar_rules` operation numbers.
fn scalar_rules_op(op: u32) -> Fp {
    match op {
        1 => Fp::D(Fp63Op::Fadd),
        2 => Fp::D(Fp63Op::Fsub),
        3 => Fp::D(Fp63Op::Fmul),
        4 => Fp::D(Fp63Op::Fdiv),
        5 => Fp::D(Fp63Op::Fsqrt),
        6 => Fp::D(Fp63Op::Fmadd),
        7 => Fp::S(Fp59Op::Fadds),
        8 => Fp::S(Fp59Op::Fmuls),
        9 => Fp::D(Fp63Op::Frsp),
        op => panic!("ppu_fp_scalar_rules names no operation {op}"),
    }
}

/// The `ppu_fp_status_writeback` operation numbers.
fn status_writeback_op(op: u32) -> Fp {
    match op {
        1 => Fp::D(Fp63Op::Fadd),
        2 => Fp::D(Fp63Op::Fsub),
        3 => Fp::D(Fp63Op::Fmul),
        4 => Fp::D(Fp63Op::Fdiv),
        5 => Fp::D(Fp63Op::Fsqrt),
        6 => Fp::D(Fp63Op::Fmadd),
        7 => Fp::D(Fp63Op::Fmsub),
        8 => Fp::D(Fp63Op::Fnmadd),
        9 => Fp::D(Fp63Op::Fnmsub),
        10 => Fp::S(Fp59Op::Fadds),
        11 => Fp::S(Fp59Op::Fsubs),
        12 => Fp::S(Fp59Op::Fmuls),
        13 => Fp::S(Fp59Op::Fdivs),
        14 => Fp::S(Fp59Op::Fsqrts),
        15 => Fp::S(Fp59Op::Fmadds),
        16 => Fp::S(Fp59Op::Fres),
        17 => Fp::D(Fp63Op::Frsqrte),
        op => panic!("ppu_fp_status_writeback names no operation {op}"),
    }
}

#[test]
fn every_status_writeback_case_matches_the_console() {
    let mut compared = 0;
    for (k, case) in cases("ppu_fp_status_writeback", 57).iter().enumerate() {
        let op = status_writeback_op(case.op);
        let (frt, fpscr) = run(op, case.word1, case.a, case.b, case.c);
        let tag = format!(
            "case {k}: {op:?} from {:#010x} on {:#018x} {:#018x} {:#018x}",
            case.word1, case.a, case.b, case.c
        );
        assert_eq!(frt, case.frt, "{tag}: FRT");
        assert_eq!(
            fpscr, case.fpscr,
            "{tag}: CellGov FPSCR {fpscr:#010x}, console {:#010x}",
            case.fpscr
        );
        compared += 1;
    }
    assert_eq!(compared, 57);
}

/// `ppu_fp_scalar_rules` cases of frsp on a NaN.
const SCALAR_FRSP: &[usize] = &[20, 21];

#[test]
fn every_scalar_rules_case_but_frsp_matches_the_console() {
    let mut compared = 0;
    for (k, case) in cases("ppu_fp_scalar_rules", 51).iter().enumerate() {
        if SCALAR_FRSP.contains(&k) {
            continue;
        }
        let op = scalar_rules_op(case.op);
        let (frt, fpscr) = run(op, case.word1, case.a, case.b, case.c);
        let tag = format!(
            "case {k}: {op:?} RN {} on {:#018x} {:#018x} {:#018x}",
            case.word1, case.a, case.b, case.c
        );
        assert_eq!(frt, case.frt, "{tag}: FRT");
        assert_eq!(
            fpscr, case.fpscr,
            "{tag}: CellGov FPSCR {fpscr:#010x}, console {:#010x}",
            case.fpscr
        );
        compared += 1;
    }
    assert_eq!(compared, 49);
}

/// Until frsp runs on the soft-float engine, it writes no status. The
/// console sets FPRF for a NaN and VXSNAN for an SNaN.
#[test]
fn frsp_of_a_nan_writes_no_status_where_the_console_sets_fprf() {
    let all = cases("ppu_fp_scalar_rules", 51);
    for &k in SCALAR_FRSP {
        let case = &all[k];
        assert_ne!(case.fpscr, 0, "case {k}: console FPSCR");
        let (frt, fpscr) = run(scalar_rules_op(case.op), case.word1, case.a, case.b, case.c);
        assert_eq!(frt, case.frt, "case {k}: FRT");
        assert_eq!(fpscr, case.word1, "case {k}: CellGov FPSCR");
    }
}
