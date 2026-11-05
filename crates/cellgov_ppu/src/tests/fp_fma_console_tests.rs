//! The multiply-add forms, double and single, held to the FRT and FPSCR
//! words of the `ppu_fp_fma_modes` and `ppu_fp_fma_range_order` console
//! captures.

use super::fp_status_console_tests::{cases, run, Fp};
use crate::instruction::ops::{Fp59Op, Fp63Op};

/// The operation numbers of the multiply-add probes.
fn fma_op(op: u32) -> Fp {
    match op {
        6 => Fp::D(Fp63Op::Fmadd),
        7 => Fp::D(Fp63Op::Fmsub),
        8 => Fp::D(Fp63Op::Fnmadd),
        9 => Fp::D(Fp63Op::Fnmsub),
        15 => Fp::S(Fp59Op::Fmadds),
        18 => Fp::S(Fp59Op::Fmsubs),
        19 => Fp::S(Fp59Op::Fnmadds),
        20 => Fp::S(Fp59Op::Fnmsubs),
        op => panic!("the multiply-add probes name no operation {op}"),
    }
}

/// Asserts that CellGov gives the FRT and FPSCR words of each of the
/// `count` cases of `probe`'s capture.
fn assert_matches_console(probe: &str, count: usize) {
    let mut differ = Vec::new();
    let all = cases(probe, count);
    for (k, case) in all.iter().enumerate() {
        let op = fma_op(case.op);
        let (frt, fpscr) = run(op, case.word1, case.a, case.b, case.c);
        if (frt, fpscr) != (case.frt, case.fpscr) {
            differ.push(format!(
                "case {k}: {op:?} from {:#010x} on {:#018x} {:#018x} {:#018x}: \
                 CellGov {frt:#018x} {fpscr:#010x}, console {:#018x} {:#010x}",
                case.word1, case.a, case.b, case.c, case.frt, case.fpscr
            ));
        }
    }
    assert_eq!(all.len(), count);
    assert!(differ.is_empty(), "{}", differ.join("\n"));
}

#[test]
fn every_multiply_add_case_matches_the_console() {
    assert_matches_console("ppu_fp_fma_modes", 61);
}

#[test]
fn every_out_of_range_single_case_matches_the_console() {
    assert_matches_console("ppu_fp_fma_range_order", 18);
}
