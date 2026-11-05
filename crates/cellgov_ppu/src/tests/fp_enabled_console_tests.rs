//! The FP arms under the FPSCR exception enables, held to the
//! `ppu_fp_enabled_*` console captures.

use super::fp_status_console_tests::{cases, Fp};
use crate::fp::{execute_fp59, execute_fp63};
use crate::instruction::ops::{Fp59Op, Fp63Op};
use crate::state::PpuState;

/// The value `ppu/main.c` loads into FRT before each case.
const FRT_BEFORE: u64 = 0x1234_5678_1122_3344;

/// The operation numbers the `ppu_fp_enabled_*` probes share.
fn enabled_results_op(op: u32) -> Fp {
    match op {
        1 => Fp::D(Fp63Op::Fadd),
        2 => Fp::D(Fp63Op::Fsub),
        3 => Fp::D(Fp63Op::Fmul),
        4 => Fp::D(Fp63Op::Fdiv),
        5 => Fp::D(Fp63Op::Fsqrt),
        6 => Fp::D(Fp63Op::Fmadd),
        10 => Fp::S(Fp59Op::Fadds),
        12 => Fp::S(Fp59Op::Fmuls),
        13 => Fp::S(Fp59Op::Fdivs),
        14 => Fp::S(Fp59Op::Fsqrts),
        21 => Fp::D(Fp63Op::Frsp),
        22 => Fp::D(Fp63Op::Fctiw),
        23 => Fp::D(Fp63Op::Fctid),
        24 => Fp::S(Fp59Op::Fres),
        25 => Fp::D(Fp63Op::Frsqrte),
        26 => Fp::D(Fp63Op::Fcmpo),
        op => panic!("the ppu_fp_enabled probes name no operation {op}"),
    }
}

/// FRT and the FPSCR after `op` runs from FPSCR `pre`.
///
/// FRA is f1, FRB is f2, FRC is f3, and FRT is f0, which holds
/// [`FRT_BEFORE`]. A compare writes CR field 1, as the probe's
/// `fcmpo cr1` does.
fn run(op: Fp, pre: u32, a: u64, b: u64, c: u64) -> (u64, u32) {
    let mut s = PpuState::new();
    s.set_fpscr(pre);
    s.set_fpr(0, FRT_BEFORE);
    s.set_fpr(1, a);
    s.set_fpr(2, b);
    s.set_fpr(3, c);
    let frt = match op {
        Fp::D(Fp63Op::Fcmpo) => 1 << 2,
        _ => 0,
    };
    match op {
        Fp::D(op) => execute_fp63(&mut s, op, frt, 1, 2, 3),
        Fp::S(op) => execute_fp59(&mut s, op, frt, 1, 2, 3),
    };
    (s.fpr[0], s.fpscr())
}

/// [PPC-Book1 p:97 s:4.4.1] with VE = 1 an invalid operation leaves FRT and FPRF unchanged and sets FR and FI to zero; a compare leaves FR, FI and C unchanged.
/// [PPC-Book1 p:98 s:4.4.2] with ZE = 1 a zero divide leaves FRT and FPRF unchanged and sets FR and FI to zero.
/// [PPC-Book1 p:98 s:4.4.3] with OE = 1 an overflowed result has its exponent reduced by 1536, or 192 for a single result and frsp, then rounded.
/// [PPC-Book1 p:99 s:4.4.4] with UE = 1 a tiny result has its exponent increased by the same amounts, then rounded, and sets UX.
/// [PPC-Book1 p:99 s:4.4.5] XE changes only FEX.
#[test]
fn every_enabled_results_case_matches_the_console() {
    assert_matches_the_console("ppu_fp_enabled_results", 56);
}

/// An fdivs or fsqrts operand outside the single range makes the
/// operation round at double precision. Then:
///
/// - an enabled overflow or underflow adjusts the exponent by 1536;
/// - a result outside the single range but inside the double range
///   raises neither.
#[test]
fn every_enabled_divsqrt_range_case_matches_the_console() {
    assert_matches_the_console("ppu_fp_enabled_divsqrt_range", 16);
}

#[test]
fn every_enabled_fres_case_matches_the_console() {
    assert_matches_the_console("ppu_fp_enabled_fres", 13);
}

/// Runs every case of `probe`'s capture and compares FRT and the FPSCR
/// with the words the console stored.
fn assert_matches_the_console(probe: &str, count: usize) {
    let mut compared = 0;
    for (k, case) in cases(probe, count).iter().enumerate() {
        let op = enabled_results_op(case.op);
        let (frt, fpscr) = run(op, case.word1, case.a, case.b, case.c);
        let tag = format!(
            "{probe} case {k}: {op:?} from {:#010x} on {:#018x} {:#018x} {:#018x}",
            case.word1, case.a, case.b, case.c
        );
        assert_eq!(
            frt, case.frt,
            "{tag}: FRT {frt:#018x}, console {:#018x}",
            case.frt
        );
        assert_eq!(
            fpscr, case.fpscr,
            "{tag}: CellGov FPSCR {fpscr:#010x}, console {:#010x}",
            case.fpscr
        );
        compared += 1;
    }
    assert_eq!(compared, count);
}
