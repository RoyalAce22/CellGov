//! `fres` and `frsqrte`: the special-operand table, NaN handling, and
//! the documented precision bound for an ordinary operand.

use super::*;
use crate::instruction::ops::{Fp59Op, Fp63Op};
use cellgov_ps3_abi::hw::ppc_isa::PPC_F64_QUIET_BIT;

const POS_ZERO: u64 = 0x0000_0000_0000_0000;
const NEG_ZERO: u64 = 0x8000_0000_0000_0000;
const POS_INF: u64 = 0x7FF0_0000_0000_0000;
const NEG_INF: u64 = 0xFFF0_0000_0000_0000;
/// A QNaN with a payload and the sign bit set.
const QNAN: u64 = 0xFFF8_0000_0000_1234;
/// An SNaN with a payload.
const SNAN: u64 = 0x7FF0_0000_0000_5678;
/// The bits FRT holds before the instruction.
const STALE: u64 = 0x0123_4567_89AB_CDEF;

/// Runs `op` with FRB = `frb` and NaNs parked in the reserved FRA and
/// FRC registers, and returns FRT.
fn run(op: Op, frb: u64) -> u64 {
    let mut s = PpuState::new();
    s.set_fpr(0, STALE);
    s.set_fpr(1, 0x7FF8_0000_0000_00AA);
    s.set_fpr(2, frb);
    s.set_fpr(3, 0x7FF8_0000_0000_00CC);
    match op {
        Op::Fres => execute_fp59(&mut s, Fp59Op::Fres, 0, 1, 2, 3),
        Op::Frsqrte => execute_fp63(&mut s, Fp63Op::Frsqrte, 0, 1, 2, 3),
    };
    s.fpr[0]
}

#[derive(Debug, Clone, Copy)]
enum Op {
    Fres,
    Frsqrte,
}

#[test]
fn fres_answers_every_special_operand_row() {
    // [PPC-Book1 p:125 s:5.2.1.1] fres special operands.
    let rows = [
        (NEG_INF, NEG_ZERO),
        (NEG_ZERO, NEG_INF),
        (POS_ZERO, POS_INF),
        (POS_INF, POS_ZERO),
        (SNAN, SNAN | PPC_F64_QUIET_BIT),
        (QNAN, QNAN),
    ];
    for (operand, want) in rows {
        assert_eq!(run(Op::Fres, operand), want, "fres 0x{operand:016x}");
    }
}

#[test]
fn frsqrte_answers_every_special_operand_row() {
    // [PPC-Book1 p:126 s:5.2.1.1] frsqrte special operands.
    let generated = cellgov_ps3_abi::hw::ppc_isa::PPC_GENERATED_QNAN_F64;
    let rows = [
        (NEG_INF, generated),
        // -1.0
        (0xBFF0_0000_0000_0000, generated),
        // the negative denormal nearest zero
        (0x8000_0000_0000_0001, generated),
        (NEG_ZERO, NEG_INF),
        (POS_ZERO, POS_INF),
        (POS_INF, POS_ZERO),
        (SNAN, SNAN | PPC_F64_QUIET_BIT),
        (QNAN, QNAN),
    ];
    for (operand, want) in rows {
        assert_eq!(run(Op::Frsqrte, operand), want, "frsqrte 0x{operand:016x}");
    }
}

#[test]
fn an_ordinary_operand_gets_an_estimate_within_the_documented_bound() {
    // [PPC-Book1 p:125 s:5.2.1.1] fres is within one part in 256; [PPC-Book1 p:126 s:5.2.1.1] frsqrte within one part in 32.
    let mut checked = 0;
    for exponent in [-100i32, -20, -1, 0, 1, 20, 100] {
        for fraction in [
            0u64,
            1,
            0x8_0000_0000_0000,
            0xF_FFFF_FFFF_FFFF,
            0x5_5555_5555_5555,
        ] {
            let bits = (((exponent + 1023) as u64) << 52) | fraction;
            let x = f64::from_bits(bits);
            let fres = f64::from_bits(run(Op::Fres, bits));
            assert!(
                ((fres - 1.0 / x) / (1.0 / x)).abs() <= 1.0 / 256.0,
                "fres {x}"
            );
            assert_eq!(f64::from(fres as f32), fres, "fres {x} is single precision");
            let frsqrte = f64::from_bits(run(Op::Frsqrte, bits));
            let exact = 1.0 / x.sqrt();
            assert!(
                ((frsqrte - exact) / exact).abs() <= 1.0 / 32.0,
                "frsqrte {x}"
            );
            checked += 1;
        }
    }
    assert_eq!(checked, 35);
}

#[test]
fn fres_of_a_double_below_single_range_overflows_to_infinity() {
    // 2^-200 has a reciprocal 2^200, past the largest single.
    let tiny = (1023u64 - 200) << 52;
    assert_eq!(run(Op::Fres, tiny), POS_INF);
    assert_eq!(run(Op::Fres, tiny | NEG_ZERO), NEG_INF);
}

#[test]
fn both_estimates_write_frt() {
    for op in [Op::Fres, Op::Frsqrte] {
        assert_ne!(run(op, 0x3FF0_0000_0000_0000), STALE, "{op:?}");
    }
}
