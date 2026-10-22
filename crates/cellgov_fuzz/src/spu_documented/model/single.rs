//! The single-precision model: the extended-range operand and result,
//! fi, and the float-to-integer conversion.

use cellgov_ps3_abi::hw::spu_fpscr::FPSCR_SINGLE_FIRST;

use super::effect::{
    push_write, word_mask, Effect, Flag, FlagWrite, Insn, Known, CLEAR, SET, SINGLE_FLAGS,
};
use crate::spu_documented::dyadic::{Dyadic, Natural};
use crate::spu_documented::question::Question;

/// A single-precision slot: its result word and its three flags.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct SingleOut {
    pub(crate) bits: Known<u32>,
    pub(crate) overflow: Flag,
    pub(crate) underflow: Flag,
    pub(crate) diff: Flag,
}

impl SingleOut {
    pub(super) fn open(question: Question) -> Self {
        let open = Known::Open(question);
        Self {
            bits: Known::Open(question),
            overflow: open,
            underflow: open,
            diff: open,
        }
    }
}

pub(super) fn single_slot(effect: &mut Effect, insn: Insn, slot: usize, out: SingleOut) {
    let value = match out.bits {
        Known::Value(bits) => Known::Value(u128::from(bits) << (96 - 32 * slot)),
        Known::Open(q) => Known::Open(q),
    };
    push_write(effect, insn, word_mask(slot), format!("w{slot}"), value);
    let first = FPSCR_SINGLE_FIRST[slot];
    for (offset, (name, raised)) in [
        ("ovf", out.overflow),
        ("unf", out.underflow),
        ("diff", out.diff),
    ]
    .into_iter()
    .enumerate()
    {
        effect.flags.push(FlagWrite {
            name: format!("s{slot}.{name}"),
            bit: first + offset as u32,
            raised,
            citation: SINGLE_FLAGS,
        });
    }
}

/// A single-precision register word as an extended-range operand, and
/// whether it sets DIFF as an input.
///
/// [SPU-ISA p:195 s:9.1] exponent 255 is an ordinary binade; [SPU-ISA p:196 s:9.1] a denorm operand is a zero, and DIFF marks an input with a maximal exponent or with a zero exponent and a nonzero fraction.
pub(crate) fn extended_operand(word: u32) -> (Dyadic, bool) {
    let exponent = word >> 23 & 0xFF;
    let fraction = word & 0x7F_FFFF;
    if exponent == 0 {
        return (Dyadic::zero(false), fraction != 0);
    }
    (
        Dyadic::new(
            word >> 31 == 1,
            u128::from(fraction | 0x80_0000),
            i64::from(exponent) - 127 - 23,
        ),
        exponent == 255,
    )
}

/// An exact result in extended-range single precision.
///
/// [SPU-ISA p:195 s:9.1] a zero result is +0, and a magnitude above the largest number gives the largest number with the sign.
/// [SPU-ISA p:196 s:9.1] truncation is the only rounding; OVF above Smax before rounding saturates to Smax, UNF below Smin gives +0, and DIFF marks a maximal-exponent result and an underflow.
pub(crate) fn extended_result(exact: &Dyadic, input_diff: bool) -> SingleOut {
    let input_diff = Known::Value(input_diff);
    if exact.is_zero() {
        return SingleOut {
            bits: Known::Value(0),
            overflow: CLEAR,
            underflow: CLEAR,
            diff: input_diff,
        };
    }
    let sign = if exact.negative { 0x8000_0000 } else { 0 };
    let leading = exact.leading_exponent();
    if leading >= 129 {
        return SingleOut {
            bits: Known::Value(sign | 0x7FFF_FFFF),
            overflow: SET,
            underflow: CLEAR,
            diff: SET,
        };
    }
    if leading < -126 {
        return SingleOut {
            bits: Known::Value(0),
            overflow: CLEAR,
            underflow: SET,
            diff: SET,
        };
    }
    let (kept, dropped) = exact.scaled(23 - leading).truncated_magnitude();
    let kept = kept
        .to_u128()
        .expect("invariant: a truncated significand has 24 bits") as u32;
    let biased = (leading + 127) as u32;
    let overflow = if leading == 128 && kept == 0xFF_FFFF && dropped {
        Known::Open(Question::SingleOverflowBand)
    } else {
        CLEAR
    };
    SingleOut {
        bits: Known::Value(sign | biased << 23 | kept & 0x7F_FFFF),
        overflow,
        underflow: CLEAR,
        diff: if biased == 255 { SET } else { input_diff },
    }
}

/// `fi` of RA word `a` and RB word `b`.
///
/// [SPU-ISA p:219 s:9] RT = (-1)^S x (1.BaseFraction - 0.000StepFraction x Y) x 2^(BiasedExponent - 127), Y = `0.RA[13:31]`, with RB laid out as on page 215; [SPU-ISA p:196 s:9.1] fi truncates and sets OVF, UNF and DIFF.
pub(crate) fn interpolate(a: u32, b: u32) -> SingleOut {
    let exponent = b >> 23 & 0xFF;
    if exponent == 0 {
        return SingleOut::open(Question::FiOperandFields);
    }
    let base = u128::from(b >> 10 & 0x1FFF);
    let step = u128::from(b & 0x3FF);
    let y = u128::from(a & 0x7_FFFF);
    // In units of 2^-32: 1.BaseFraction is (2^13 + base) x 2^19, and
    // 0.000StepFraction x Y is step x y.
    let value = ((0x2000 | base) << 19) - step * y;
    let exact = Dyadic::new(b >> 31 == 1, value, i64::from(exponent) - 127 - 32);
    let mut out = extended_result(&exact, exponent == 255);
    let ra_exponent = a >> 23 & 0xFF;
    let ra_reads_as_flagged = ra_exponent == 255 || ra_exponent == 0 && a & 0x7F_FFFF != 0;
    if ra_reads_as_flagged && out.diff == CLEAR {
        out.diff = Known::Open(Question::FiOperandFields);
    }
    out
}

/// cflts (`signed`) or cfltu of one word at `scale`.
///
/// [SPU-ISA p:221 s:9] the value times 2^scale, saturated to [-2^31, 2^31 - 1]; [SPU-ISA p:223 s:9] cfltu saturates to [0, 2^32 - 1]; [SPU-ISA p:196 s:9.1] truncation is the only rounding and a denorm is a zero.
pub(crate) fn to_integer(word: u32, scale: i64, signed: bool) -> u32 {
    let (value, _) = extended_operand(word);
    let (magnitude, _) = value.scaled(scale).truncated_magnitude();
    let limit =
        |bound: u128| magnitude.cmp(&Natural::from_u128(bound)) == std::cmp::Ordering::Greater;
    match (signed, value.negative && !value.is_zero()) {
        (true, false) if limit(0x7FFF_FFFF) => 0x7FFF_FFFF,
        (true, true) if limit(0x8000_0000) => 0x8000_0000,
        (true, negative) => {
            let m = magnitude
                .to_u128()
                .expect("invariant: checked against 2^31") as u32;
            if negative {
                m.wrapping_neg()
            } else {
                m
            }
        }
        (false, true) => 0,
        (false, false) if limit(0xFFFF_FFFF) => 0xFFFF_FFFF,
        (false, false) => magnitude
            .to_u128()
            .expect("invariant: checked against 2^32") as u32,
    }
}
