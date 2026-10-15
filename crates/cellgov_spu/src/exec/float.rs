//! Single-precision arithmetic on the `cellgov_float` engine. For each
//! word slot the engine decodes the operands, combines them exactly, then
//! truncates and packs the result; the slot's flags OR into its FPSCR
//! bits. The arms hold no floating-point logic.

use cellgov_float::{
    round_pack, unpack_extended, Binary32, Exact, Flags, Format, Policy, Rounding,
};

use super::lanes::{from_words, words};
use cellgov_ps3_abi::hw::spu_isa::{TO_FLOAT_SCALE_BIAS, TO_INTEGER_SCALE_BIAS};

use super::outcome::{SpuFault, SpuStepOutcome};
use crate::state::SpuState;

/// Applies `op` to each word slot of the `sources` registers as
/// single-precision values, writes the packed results to `rt`, and ORs
/// each slot's flags into the FPSCR.
///
/// [SPU-ISA p:196 s:9.1] fa, fs, fm, fma, fms and fnms set OVF, UNF and DIFF in the slice of each slot.
pub(super) fn single<const N: usize>(
    state: &mut SpuState,
    rt: u8,
    sources: [u8; N],
    op: impl Fn([Exact; N]) -> Exact,
) -> SpuStepOutcome {
    let words_of = sources.map(|r| words(state.regs[r as usize]));
    let mut flags = [Flags::default(); 4];
    let results = std::array::from_fn(|slot| {
        let mut operand_flags = Flags::default();
        let operands = words_of.map(|w| {
            let (x, x_flags) = unpack_extended::<Binary32>(u64::from(w[slot]));
            operand_flags = operand_flags.or(x_flags);
            x
        });
        // The extended policy truncates whatever mode is named.
        let packed =
            round_pack::<Binary32>(Policy::SpuExtended, Rounding::TowardZero, op(operands));
        flags[slot] = operand_flags.or(packed.flags);
        packed.bits as u32
    });
    state.set_reg(rt as usize, from_words(results));
    state.fpscr_accumulate_single(flags);
    SpuStepOutcome::Continue
}

/// Applies `op` to each word slot of `ra`, writes the results to `rt`,
/// and sets DBZ for each slot whose operand has a zero exponent.
///
/// [SPU-ISA p:215 s:9] and [SPU-ISA p:217 s:9]: a zero exponent flags divide by zero; [SPU-ISA p:196 s:9.1] frest and frsqest set DBZ only.
pub(super) fn estimate(state: &mut SpuState, rt: u8, ra: u8, op: fn(u32) -> u32) -> SpuStepOutcome {
    let a = words(state.regs[ra as usize]);
    state.set_reg(rt as usize, from_words(a.map(op)));
    state.fpscr_accumulate_dbz(a.map(|x| x >> 23 & 0xFF == 0));
    SpuStepOutcome::Continue
}

/// `fi` on each word slot: RB's base less its step times RA's fraction,
/// truncated to extended-range single precision.
///
/// [SPU-ISA p:219 s:9] RT = (-1)^S x (1.BaseFraction - 0.000StepFraction x Y) x 2^(BiasedExponent - 127), Y = 0.`RA[13:31]`; [SPU-ISA p:196 s:9.1] fi sets OVF, UNF and DIFF.
///
/// [SPU-ISA p:196 s:9.1] a zero-exponent operand is a zero, and DIFF marks one with a nonzero fraction and one with exponent 255. RB is fi's one floating operand: the `spu_fi_zero_operand` capture gives +0 for an exponent-0 RB of either sign, DIFF only with a nonzero fraction, and no flag from RA's exponent.
pub(super) fn interpolate(state: &mut SpuState, rt: u8, ra: u8, rb: u8) -> SpuStepOutcome {
    let [a, b] = [ra, rb].map(|r| words(state.regs[r as usize]));
    let mut flags = [Flags::default(); 4];
    let results = std::array::from_fn(|slot| {
        let (y, packed_estimate) = (a[slot] & 0x7_FFFF, b[slot]);
        let (_, operand_flags) = unpack_extended::<Binary32>(u64::from(packed_estimate));
        let exponent = (packed_estimate >> 23 & 0xFF) as i32;
        if exponent == 0 {
            flags[slot] = operand_flags;
            return 0;
        }
        let base = packed_estimate >> 10 & 0x1FFF;
        let step = packed_estimate & 0x3FF;
        // In units of 2^-32: 1.BaseFraction is (2^13 + base) x 2^19 and
        // 0.000StepFraction x Y is step x y, so the value is exact.
        let value = (u128::from(0x2000 | base) << 19) - u128::from(step) * u128::from(y);
        let exact = Exact::new(packed_estimate >> 31 == 1, value, exponent - 127 - 32)
            .expect("invariant: the interpolated significand is at most 33 bits wide");
        let packed = round_pack::<Binary32>(Policy::SpuExtended, Rounding::TowardZero, exact);
        flags[slot] = operand_flags.or(packed.flags);
        packed.bits as u32
    });
    state.set_reg(rt as usize, from_words(results));
    state.fpscr_accumulate_single(flags);
    SpuStepOutcome::Continue
}

/// The scale `bias - imm`, for any I8.
///
/// [SPU-ISA p:220 s:9] and [SPU-ISA p:221 s:9] leave a scale outside 0..=127 undefined. The `spu_convert_scales` and `spu_convert_sweep` captures give the console's answer: csflt and cuflt follow their formula at every scale, and so do cflts and cfltu apart from the exponent edges `to_integer` names.
fn scale(bias: u8, imm: u8) -> i32 {
    i32::from(bias) - i32::from(imm)
}

/// The biased exponent plus the scale at which cflts (`signed`) or
/// cfltu saturates: the value reaches 2^31 or 2^32.
fn saturation_point(signed: bool) -> i32 {
    Binary32::BIAS + if signed { 31 } else { 32 }
}

/// Whether cflts (`signed`) or cfltu at this I8 refuses some input:
/// whether the largest biased exponent reaches 256 past the saturation
/// point.
pub(crate) fn to_integer_can_refuse(imm: u8, signed: bool) -> bool {
    255 + scale(TO_INTEGER_SCALE_BIAS, imm) - saturation_point(signed) >= 256
}

/// `csflt` / `cuflt`: each slot's integer divided by 2^scale, truncated
/// to extended-range single precision.
///
/// [SPU-ISA p:196 s:9.1] csflt and cuflt set OVF, UNF and DIFF, truncating.
pub(super) fn to_float(
    state: &mut SpuState,
    rt: u8,
    ra: u8,
    imm: u8,
    signed: bool,
) -> SpuStepOutcome {
    let scale = scale(TO_FLOAT_SCALE_BIAS, imm);
    let a = words(state.regs[ra as usize]);
    let mut flags = [Flags::default(); 4];
    let results = std::array::from_fn(|slot| {
        let (negative, magnitude) = if signed {
            let value = a[slot] as i32;
            (value < 0, value.unsigned_abs())
        } else {
            (false, a[slot])
        };
        let exact = Exact::new(negative, u128::from(magnitude), -scale)
            .expect("invariant: a 32-bit integer fits the exact significand");
        let packed = round_pack::<Binary32>(Policy::SpuExtended, Rounding::TowardZero, exact);
        flags[slot] = packed.flags;
        packed.bits as u32
    });
    state.set_reg(rt as usize, from_words(results));
    state.fpscr_accumulate_single(flags);
    SpuStepOutcome::Continue
}

/// `cflts` / `cfltu`: each slot's value times 2^scale, truncated toward
/// zero and saturated to the integer range.
///
/// [SPU-ISA p:221 s:9] cflts saturates above 2^31 - 1 and below -2^31; [SPU-ISA p:223 s:9] cfltu saturates above 2^32 - 1 and every negative product to zero; [SPU-ISA p:196 s:9.1] truncation is the only single-precision rounding, and neither sets a flag.
///
/// Only an undefined scale reaches two edges, which the
/// `spu_convert_sweep` capture shows:
///
/// - A slot whose biased exponent plus the scale reaches the saturation
///   point saturates even when its exponent is 0, a zero for every
///   other purpose.
/// - At 256 or more past that point the console's result for a slot
///   depends on more than the slot's input and the scale: the same
///   instruction and input gave different results in two probes. Such
///   a slot refuses the instruction.
pub(super) fn to_integer(
    state: &mut SpuState,
    rt: u8,
    ra: u8,
    imm: u8,
    signed: bool,
) -> SpuStepOutcome {
    let scale = scale(TO_INTEGER_SCALE_BIAS, imm);
    let a = words(state.regs[ra as usize]);
    let past_saturation = a.map(|word| {
        (word >> Binary32::FRAC_BITS & Binary32::EXP_MAX) as i32 + scale - saturation_point(signed)
    });
    if past_saturation.iter().any(|&past| past >= 256) {
        return SpuStepOutcome::Fault(SpuFault::UndefinedConversionScale(imm));
    }
    let results = std::array::from_fn(|slot| {
        let word = a[slot];
        let (x, _) = unpack_extended::<Binary32>(u64::from(word));
        // The magnitude truncated toward zero, or `None` at 2^33 and above.
        let shift = x.exponent() + scale;
        let significand = x.significand();
        let magnitude = if past_saturation[slot] >= 0 {
            None
        } else if significand == 0 {
            Some(0)
        } else if shift >= 0 {
            (128 - significand.leading_zeros() as i32 + shift <= 33).then(|| significand << shift)
        } else {
            Some(significand.checked_shr(shift.unsigned_abs()).unwrap_or(0))
        };
        match (signed, word >> 31 == 1, magnitude) {
            (true, false, Some(m)) if m <= i32::MAX as u128 => m as u32,
            (true, false, _) => i32::MAX as u32,
            (true, true, Some(m)) if m <= 1 << 31 => (m as u32).wrapping_neg(),
            (true, true, _) => i32::MIN as u32,
            (false, false, Some(m)) if m <= u128::from(u32::MAX) => m as u32,
            (false, false, _) => u32::MAX,
            (false, true, _) => 0,
        }
    });
    state.set_reg(rt as usize, from_words(results));
    SpuStepOutcome::Continue
}

/// Writes all ones to each word slot of `rt` where `holds` is true of the
/// slot's `ra` and `rb` words, and zero elsewhere.
///
/// [SPU-ISA p:196 s:9.1] the compares set no flag.
pub(super) fn compare(
    state: &mut SpuState,
    rt: u8,
    ra: u8,
    rb: u8,
    holds: impl Fn(u32, u32) -> bool,
) -> SpuStepOutcome {
    let [a, b] = [ra, rb].map(|r| words(state.regs[r as usize]));
    state.set_reg(
        rt as usize,
        from_words(std::array::from_fn(|slot| {
            if holds(a[slot], b[slot]) {
                u32::MAX
            } else {
                0
            }
        })),
    );
    SpuStepOutcome::Continue
}

/// `a + b` for two exact operands: decoded single-precision values or
/// one of their products.
pub(super) fn sum(a: Exact, b: Exact) -> Exact {
    cellgov_float::add(a, b)
        .expect("invariant: single-precision operands and their products are exact and at most 48 bits wide")
}

/// `a * b` for two decoded single-precision operands.
pub(super) fn product(a: Exact, b: Exact) -> Exact {
    cellgov_float::mul(a, b)
        .expect("invariant: decoded single-precision operands are exact and 24 bits wide")
}
