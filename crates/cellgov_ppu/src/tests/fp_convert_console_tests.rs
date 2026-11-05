//! The conversions held to the FRT and FPSCR words of the
//! `ppu_fp_convert_modes` console capture.

use super::fp_status_console_tests::{cases, run, Fp};
use crate::instruction::ops::Fp63Op;

/// The operation numbers of the `ppu_fp_convert_modes` probe.
fn convert_op(op: u32) -> Fp {
    Fp::D(match op {
        1 => Fp63Op::Fctiw,
        2 => Fp63Op::Fctiwz,
        3 => Fp63Op::Fctid,
        4 => Fp63Op::Fctidz,
        5 => Fp63Op::Fcfid,
        6 => Fp63Op::Frsp,
        op => panic!("ppu_fp_convert_modes names no operation {op}"),
    })
}

#[test]
fn every_conversion_case_matches_the_console() {
    let mut differ = Vec::new();
    let all = cases("ppu_fp_convert_modes", 153);
    for (k, case) in all.iter().enumerate() {
        let op = convert_op(case.op);
        let (frt, fpscr) = run(op, case.word1, case.a, case.b, case.c);
        if (frt, fpscr) != (case.frt, case.fpscr) {
            differ.push(format!(
                "case {k}: {op:?} from {:#010x} on {:#018x}: \
                 CellGov {frt:#018x} {fpscr:#010x}, console {:#018x} {:#010x}",
                case.word1, case.b, case.frt, case.fpscr
            ));
        }
    }
    assert_eq!(all.len(), 153);
    assert!(differ.is_empty(), "{}", differ.join("\n"));
}
