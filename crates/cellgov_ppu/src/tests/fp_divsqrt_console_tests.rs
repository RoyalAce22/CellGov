//! `fdiv`, `fdivs`, `fsqrt` and `fsqrts` held to the
//! `ppu_fp_divsqrt_modes` capture from the reference console. Each case
//! runs the probe's one instruction on the probe's operands, from the
//! probe's FPSCR, and compares FRT and the FPSCR with the words the
//! console stored.

use super::fp_status_console_tests::{cases, run, Fp};
use crate::instruction::ops::{Fp59Op, Fp63Op};

/// The `ppu_fp_divsqrt_modes` operation numbers.
fn divsqrt_op(op: u32) -> Fp {
    match op {
        4 => Fp::D(Fp63Op::Fdiv),
        5 => Fp::D(Fp63Op::Fsqrt),
        13 => Fp::S(Fp59Op::Fdivs),
        14 => Fp::S(Fp59Op::Fsqrts),
        op => panic!("ppu_fp_divsqrt_modes names no operation {op}"),
    }
}

#[test]
fn every_divide_and_square_root_case_matches_the_console() {
    let mut differ = Vec::new();
    let all = cases("ppu_fp_divsqrt_modes", 85);
    for (k, case) in all.iter().enumerate() {
        let op = divsqrt_op(case.op);
        let (frt, fpscr) = run(op, case.word1, case.a, case.b, case.c);
        if (frt, fpscr) != (case.frt, case.fpscr) {
            differ.push(format!(
                "case {k}: {op:?} from {:#010x} on {:#018x} {:#018x} {:#018x}: \
                 CellGov {frt:#018x} {fpscr:#010x}, console {:#018x} {:#010x}",
                case.word1, case.a, case.b, case.c, case.frt, case.fpscr
            ));
        }
    }
    assert_eq!(all.len(), 85);
    assert!(differ.is_empty(), "{}", differ.join("\n"));
}

/// The `ppu_fp_divsqrt_single_bits` capture: fdivs and fsqrts round the
/// exact result of the full operands once. The operands are doubles
/// inside the single exponent range, with fraction bits below single
/// precision.
#[test]
fn every_single_divide_and_root_of_a_wide_operand_matches_the_console() {
    let mut differ = Vec::new();
    let all = cases("ppu_fp_divsqrt_single_bits", 31);
    for (k, case) in all.iter().enumerate() {
        let op = match case.op {
            13 => Fp::S(Fp59Op::Fdivs),
            14 => Fp::S(Fp59Op::Fsqrts),
            op => panic!("ppu_fp_divsqrt_single_bits names no operation {op}"),
        };
        let (frt, fpscr) = run(op, case.word1, case.a, case.b, case.c);
        if (frt, fpscr) != (case.frt, case.fpscr) {
            differ.push(format!(
                "case {k}: {op:?} from {:#010x} on {:#018x} {:#018x}: \
                 CellGov {frt:#018x} {fpscr:#010x}, console {:#018x} {:#010x}",
                case.word1, case.a, case.b, case.frt, case.fpscr
            ));
        }
    }
    assert_eq!(all.len(), 31);
    assert!(differ.is_empty(), "{}", differ.join("\n"));
}

/// The `ppu_fp_sqrts_exact_tie` capture: fsqrts of the double
/// (2^24 + 1)^2 rounds an exact root that lies halfway between two
/// singles, in each mode, and fsqrt gives the exact root.
#[test]
fn a_single_root_that_is_an_exact_tie_matches_the_console() {
    let all = cases("ppu_fp_sqrts_exact_tie", 5);
    let mut differ = Vec::new();
    for (k, case) in all.iter().enumerate() {
        let op = match case.op {
            5 => Fp::D(Fp63Op::Fsqrt),
            14 => Fp::S(Fp59Op::Fsqrts),
            op => panic!("ppu_fp_sqrts_exact_tie names no operation {op}"),
        };
        let (frt, fpscr) = run(op, case.word1, case.a, case.b, case.c);
        if (frt, fpscr) != (case.frt, case.fpscr) {
            differ.push(format!(
                "case {k}: {op:?} from {:#010x} on {:#018x}: \
                 CellGov {frt:#018x} {fpscr:#010x}, console {:#018x} {:#010x}",
                case.word1, case.b, case.frt, case.fpscr
            ));
        }
    }
    assert_eq!(all.len(), 5);
    assert!(differ.is_empty(), "{}", differ.join("\n"));
}
