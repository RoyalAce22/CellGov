//! `fadd`, `fsub` and `fmul`, double and single, held to the FRT and
//! FPSCR words of the `ppu_fp_addmul_modes` and `ppu_fp_addmul_edges`
//! console captures.

use super::fp_status_console_tests::{cases, run, Fp};
use crate::instruction::ops::{Fp59Op, Fp63Op};

/// The operation numbers of the `ppu_fp_addmul_modes` and
/// `ppu_fp_addmul_edges` probes.
fn addmul_op(op: u32) -> Fp {
    match op {
        1 => Fp::D(Fp63Op::Fadd),
        2 => Fp::D(Fp63Op::Fsub),
        3 => Fp::D(Fp63Op::Fmul),
        10 => Fp::S(Fp59Op::Fadds),
        11 => Fp::S(Fp59Op::Fsubs),
        12 => Fp::S(Fp59Op::Fmuls),
        op => panic!("ppu_fp_addmul_modes names no operation {op}"),
    }
}

#[test]
fn every_add_subtract_and_multiply_case_matches_the_console() {
    let mut compared = 0;
    for (k, case) in cases("ppu_fp_addmul_modes", 97).iter().enumerate() {
        let op = addmul_op(case.op);
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
    assert_eq!(compared, 97);
}

/// The `ppu_fp_addmul_edges` capture: NaN, infinity, zero and denormal
/// operands, sticky-only sums and out-of-range single operands.
#[test]
fn every_operand_class_edge_case_matches_the_console() {
    let mut differ = Vec::new();
    let all = cases("ppu_fp_addmul_edges", 63);
    for (k, case) in all.iter().enumerate() {
        let op = addmul_op(case.op);
        let (frt, fpscr) = run(op, case.word1, case.a, case.b, case.c);
        if (frt, fpscr) != (case.frt, case.fpscr) {
            differ.push(format!(
                "case {k}: {op:?} from {:#010x} on {:#018x} {:#018x} {:#018x}: \
                 CellGov {frt:#018x} {fpscr:#010x}, console {:#018x} {:#010x}",
                case.word1, case.a, case.b, case.c, case.frt, case.fpscr
            ));
        }
    }
    assert_eq!(all.len(), 63);
    assert!(differ.is_empty(), "{}", differ.join("\n"));
}
