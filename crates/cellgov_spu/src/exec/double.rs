//! Double-precision arithmetic on the `cellgov_float` engine: IEEE 754 with
//! the CBE's deviations, one rounding per doubleword slot in the mode its
//! FPSCR slice names, and the six flags ORed into that slice.

use cellgov_float::{
    add, default_nan, mul, round_pack, unpack, Binary64, Exact, Flags, Operand, Policy, Rounding,
};

use super::lanes::{doublewords, from_doublewords};
use super::outcome::SpuStepOutcome;
use crate::state::SpuState;

/// Whether a denormal operand, read as zero with DENORM, also raises INV.
// [SPU-ISA p:199 s:9.2.2] an implementation may set INV as well as DENORM for a denormal operand; CellGov sets DENORM only until hardware vectors decide.
const DENORMAL_OPERAND_RAISES_INVALID: bool = false;

/// Whether a NaN result takes the sign of NaN operands that are all
/// negative, rather than the default QNaN's positive sign.
// [SPU-ISA p:197 s:9.2] a NaN result may be the default QNaN, sign 0, even with NaN inputs; [Mueller2005 p:61 s:3.2] the CBE computes the generic NaN for every NaN result; hardware vectors decide whether an all-negative NaN input changes the sign.
const NAN_RESULT_TAKES_INPUT_SIGN: bool = false;

/// The double-precision operation an arm applies.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum DoubleOp {
    /// `a + b`.
    Add,
    /// `a - b`.
    Subtract,
    /// `a x b`.
    Multiply,
}

/// Applies `op` to each doubleword slot of `ra` and `rb`, rounded by the
/// slot's FPSCR mode, writes the results to `rt`, and ORs each slot's flags
/// into its FPSCR slice.
// [SPU-ISA p:197 s:9.2] slice 0 rounds by RN0 and slice 1 by RN1, and no exception traps; [SPU-ISA p:200 s:9.3] each slice has its own flags.
pub(super) fn double(state: &mut SpuState, rt: u8, ra: u8, rb: u8, op: DoubleOp) -> SpuStepOutcome {
    let rounding = state.fpscr_rounding();
    let [a, b] = [ra, rb].map(|r| doublewords(state.regs[r as usize]));
    let mut flags = [Flags::default(); 2];
    let results = std::array::from_fn(|slot| {
        let (x, x_flags) = unpack::<Binary64>(Policy::Ieee754Cbe, a[slot]);
        let (y, y_flags) = unpack::<Binary64>(Policy::Ieee754Cbe, b[slot]);
        let y = if op == DoubleOp::Subtract {
            negated(y)
        } else {
            y
        };
        let (bits, result_flags) = result(op, x, y, rounding[slot], [a[slot], b[slot]]);
        let operand_flags = x_flags.or(y_flags);
        flags[slot] = operand_flags.or(result_flags).or(Flags {
            invalid: DENORMAL_OPERAND_RAISES_INVALID && operand_flags.denormal,
            ..Flags::default()
        });
        bits
    });
    state.regs[rt as usize] = from_doublewords(results);
    state.fpscr_accumulate_double(flags);
    SpuStepOutcome::Continue
}

/// `x` with its sign flipped; a NaN keeps its payload.
fn negated(x: Operand) -> Operand {
    match x {
        Operand::Zero { negative } => Operand::Zero {
            negative: !negative,
        },
        Operand::Finite(exact) => Operand::Finite(exact.negated()),
        Operand::Infinity { negative } => Operand::Infinity {
            negative: !negative,
        },
        nan @ Operand::NaN { .. } => nan,
    }
}

/// The sign of a zero, finite or infinite operand.
fn sign(x: Operand) -> bool {
    match x {
        Operand::Zero { negative } | Operand::Infinity { negative } => negative,
        Operand::Finite(exact) => exact.negative(),
        Operand::NaN { .. } => false,
    }
}

/// The exact value of a zero or finite operand.
fn exact(x: Operand) -> Exact {
    match x {
        Operand::Finite(exact) => exact,
        _ => Exact::new(sign(x), 0, 0).expect("invariant: a zero significand fits"),
    }
}

/// An infinity of the given sign.
fn infinity(negative: bool) -> u64 {
    u64::from(negative) << 63 | 0x7FF0_0000_0000_0000
}

/// The default QNaN, with INV when `invalid`.
// [SPU-ISA p:197 s:9.2] every NaN result is the default QNaN 0x7FF8000000000000; [Mueller2005 p:61 s:3.2] the CBE does not propagate an input NaN.
fn nan_result(invalid: bool, negative: bool) -> (u64, Flags) {
    let sign = u64::from(NAN_RESULT_TAKES_INPUT_SIGN && negative) << 63;
    (
        sign | default_nan::<Binary64>(),
        Flags {
            invalid,
            ..Flags::default()
        },
    )
}

/// The default-QNaN result when any operand is a NaN: INV for an SNaN,
/// and the sign switch reading the raw operand words.
fn nan_operands(operands: &[Operand], raw: &[u64]) -> Option<(u64, Flags)> {
    if !operands.iter().any(|o| matches!(o, Operand::NaN { .. })) {
        return None;
    }
    let signaling = operands
        .iter()
        .any(|o| matches!(o, Operand::NaN { quiet: false }));
    // Whether every NaN operand is negative.
    let is_raw_nan = |w: u64| w & 0x7FFF_FFFF_FFFF_FFFF > 0x7FF0_0000_0000_0000;
    let all_negative = raw.iter().filter(|w| is_raw_nan(**w)).all(|w| w >> 63 == 1);
    Some(nan_result(signaling, all_negative))
}

/// `a + b` rounded once in `rounding`. An exact zero sum is +0, or -0
/// under round toward -inf, unless both addends are zeros of one sign.
fn rounded_sum(a: Exact, b: Exact, rounding: Rounding) -> (u64, Flags) {
    let sum = add(a, b).expect("invariant: the addends are exact and at most 106 bits wide");
    if sum.significand() == 0 {
        let zeros_of_one_sign =
            a.significand() == 0 && b.significand() == 0 && a.negative() == b.negative();
        let negative = if zeros_of_one_sign {
            a.negative()
        } else {
            rounding == Rounding::TowardNegative
        };
        return (u64::from(negative) << 63, Flags::default());
    }
    let packed = round_pack::<Binary64>(Policy::Ieee754Cbe, rounding, sum);
    (packed.bits, packed.flags)
}

/// A fused multiply-add form: RA x RB with RT added, or subtracted, and
/// the rounded result optionally negated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Fused {
    /// Subtract RT from the product.
    pub(super) subtract: bool,
    /// Negate every rounded result that is not a NaN.
    pub(super) negate: bool,
}

/// Applies a fused form to each doubleword slot, with RT the addend: RT
/// the arm reads before it writes.
// [SPU-ISA p:209 s:9] dfma: RA x RB + RT, the multiplication exact and not subject to limits on its range; [SPU-ISA p:213 s:9] dfms subtracts RT.
// [SPU-ISA p:211 s:9] and [SPU-ISA p:214 s:9]: dfnms and dfnma negate the rounded result of dfms and dfma, except that a NaN result keeps sign 0.
pub(super) fn fused(state: &mut SpuState, rt: u8, ra: u8, rb: u8, form: Fused) -> SpuStepOutcome {
    let rounding = state.fpscr_rounding();
    let [a, b, c] = [ra, rb, rt].map(|r| doublewords(state.regs[r as usize]));
    let mut flags = [Flags::default(); 2];
    let results = std::array::from_fn(|slot| {
        let (x, x_flags) = unpack::<Binary64>(Policy::Ieee754Cbe, a[slot]);
        let (y, y_flags) = unpack::<Binary64>(Policy::Ieee754Cbe, b[slot]);
        let (z, z_flags) = unpack::<Binary64>(Policy::Ieee754Cbe, c[slot]);
        let z = if form.subtract { negated(z) } else { z };
        let (bits, result_flags) =
            fused_result(x, y, z, rounding[slot], [a[slot], b[slot], c[slot]]);
        let operand_flags = x_flags.or(y_flags).or(z_flags);
        flags[slot] = operand_flags.or(result_flags).or(Flags {
            invalid: DENORMAL_OPERAND_RAISES_INVALID && operand_flags.denormal,
            ..Flags::default()
        });
        let is_nan = bits & 0x7FFF_FFFF_FFFF_FFFF > 0x7FF0_0000_0000_0000;
        if form.negate && !is_nan {
            bits ^ 1 << 63
        } else {
            bits
        }
    });
    state.regs[rt as usize] = from_doublewords(results);
    state.fpscr_accumulate_double(flags);
    SpuStepOutcome::Continue
}

/// The encoded result of `x * y + z`, rounded once, and the flags it raises
/// beyond its operands'.
// [SPU-ISA p:199 s:9.2.2] INV for an SNaN operand, for infinity times zero, and for infinities of opposite signs added.
fn fused_result(
    x: Operand,
    y: Operand,
    z: Operand,
    rounding: Rounding,
    raw: [u64; 3],
) -> (u64, Flags) {
    if let Some(nan) = nan_operands(&[x, y, z], &raw) {
        return nan;
    }
    let is_infinity = |o: Operand| matches!(o, Operand::Infinity { .. });
    let is_zero = |o: Operand| matches!(o, Operand::Zero { .. });
    let none = Flags::default();
    if (is_infinity(x) && is_zero(y)) || (is_zero(x) && is_infinity(y)) {
        return nan_result(true, false);
    }
    if is_infinity(x) || is_infinity(y) {
        let product_negative = sign(x) != sign(y);
        if is_infinity(z) && sign(z) != product_negative {
            return nan_result(true, false);
        }
        return (infinity(product_negative), none);
    }
    if is_infinity(z) {
        return (infinity(sign(z)), none);
    }
    let product = mul(exact(x), exact(y))
        .expect("invariant: two 53-bit significands multiply within 106 bits");
    rounded_sum(product, exact(z), rounding)
}

/// The encoded result of `op` on `x` and `y` (already negated for a
/// subtraction), and the flags the operation raises beyond its operands'.
// [SPU-ISA p:199 s:9.2.2] INV for an SNaN operand, for infinity less infinity in magnitude, and for infinity times zero.
fn result(op: DoubleOp, x: Operand, y: Operand, rounding: Rounding, raw: [u64; 2]) -> (u64, Flags) {
    if let Some(nan) = nan_operands(&[x, y], &raw) {
        return nan;
    }
    let is_infinity = |o: Operand| matches!(o, Operand::Infinity { .. });
    let is_zero = |o: Operand| matches!(o, Operand::Zero { .. });
    let none = Flags::default();
    match op {
        DoubleOp::Add | DoubleOp::Subtract => {
            if is_infinity(x) && is_infinity(y) && sign(x) != sign(y) {
                return nan_result(true, false);
            }
            if is_infinity(x) {
                return (infinity(sign(x)), none);
            }
            if is_infinity(y) {
                return (infinity(sign(y)), none);
            }
            rounded_sum(exact(x), exact(y), rounding)
        }
        DoubleOp::Multiply => {
            if (is_infinity(x) && is_zero(y)) || (is_zero(x) && is_infinity(y)) {
                return nan_result(true, false);
            }
            if is_infinity(x) || is_infinity(y) {
                return (infinity(sign(x) != sign(y)), none);
            }
            let product = mul(exact(x), exact(y))
                .expect("invariant: two 53-bit significands multiply within 106 bits");
            let packed = round_pack::<Binary64>(Policy::Ieee754Cbe, rounding, product);
            (packed.bits, packed.flags)
        }
    }
}
