//! Single-precision arithmetic on the `cellgov_float` engine. For each
//! word slot the engine decodes the operands, combines them exactly, then
//! truncates and packs the result; the slot's flags OR into its FPSCR
//! bits. The arms hold no floating-point logic.

use cellgov_float::{round_pack, unpack_extended, Binary32, Exact, Flags, Policy, Rounding};

use super::lanes::{from_words, words};
use cellgov_ps3_abi::hw::spu_isa::{TO_FLOAT_SCALE_BIAS, TO_INTEGER_SCALE_BIAS};

use super::outcome::{SpuFault, SpuStepOutcome};
use crate::state::SpuState;

/// Applies `op` to each word slot of the `sources` registers as
/// single-precision values, writes the packed results to `rt`, and ORs
/// each slot's flags into the FPSCR.
// [SPU-ISA p:196 s:9.1] fa, fs, fm, fma, fms and fnms set OVF, UNF and DIFF in the slice of each slot.
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
    state.regs[rt as usize] = from_words(results);
    state.fpscr_accumulate_single(flags);
    SpuStepOutcome::Continue
}

/// Applies `op` to each word slot of `ra`, writes the results to `rt`,
/// and sets DBZ for each slot whose operand has a zero exponent.
// [SPU-ISA p:215 s:9] and [SPU-ISA p:217 s:9]: a zero exponent flags divide by zero; [SPU-ISA p:196 s:9.1] frest and frsqest set DBZ only.
pub(super) fn estimate(state: &mut SpuState, rt: u8, ra: u8, op: fn(u32) -> u32) -> SpuStepOutcome {
    let a = words(state.regs[ra as usize]);
    state.regs[rt as usize] = from_words(a.map(op));
    state.fpscr_accumulate_dbz(a.map(|x| x >> 23 & 0xFF == 0));
    SpuStepOutcome::Continue
}

/// `fi` on each word slot: RB's base less its step times RA's fraction,
/// truncated to extended-range single precision.
// [SPU-ISA p:219 s:9] RT = (-1)^S x (1.BaseFraction - 0.000StepFraction x Y) x 2^(BiasedExponent - 127), Y = 0.RA[13:31]; [SPU-ISA p:196 s:9.1] fi sets OVF, UNF and DIFF.
pub(super) fn interpolate(state: &mut SpuState, rt: u8, ra: u8, rb: u8) -> SpuStepOutcome {
    let [a, b] = [ra, rb].map(|r| words(state.regs[r as usize]));
    let mut flags = [Flags::default(); 4];
    let results = std::array::from_fn(|slot| {
        let (y, packed_estimate) = (a[slot] & 0x7_FFFF, b[slot]);
        let exponent = (packed_estimate >> 23 & 0xFF) as i32;
        let base = packed_estimate >> 10 & 0x1FFF;
        let step = packed_estimate & 0x3FF;
        // In units of 2^-32: 1.BaseFraction is (2^13 + base) x 2^19 and
        // 0.000StepFraction x Y is step x y, so the value is exact.
        let value = (u128::from(0x2000 | base) << 19) - u128::from(step) * u128::from(y);
        let exact = Exact::new(packed_estimate >> 31 == 1, value, exponent - 127 - 32)
            .expect("invariant: the interpolated significand is at most 33 bits wide");
        let packed = round_pack::<Binary32>(Policy::SpuExtended, Rounding::TowardZero, exact);
        // An RB exponent of 255 is an input with exponent 255.
        flags[slot] = packed.flags.or(Flags {
            diff: exponent == 255,
            ..Flags::default()
        });
        packed.bits as u32
    });
    state.regs[rt as usize] = from_words(results);
    state.fpscr_accumulate_single(flags);
    SpuStepOutcome::Continue
}

/// The scale `bias - imm`, or `None` outside 0..=127, where every
/// conversion's result is undefined.
// [SPU-ISA p:220 s:9] a scale outside 0..=127 is undefined; [SPU-ISA p:221 s:9] the same for the integer conversions.
pub(crate) fn scale(bias: u8, imm: u8) -> Option<u32> {
    let scale = i32::from(bias) - i32::from(imm);
    (0..=127).contains(&scale).then_some(scale as u32)
}

/// `csflt` / `cuflt`: each slot's integer divided by 2^scale, truncated
/// to extended-range single precision.
// [SPU-ISA p:196 s:9.1] csflt and cuflt set OVF, UNF and DIFF, truncating.
pub(super) fn to_float(
    state: &mut SpuState,
    rt: u8,
    ra: u8,
    imm: u8,
    signed: bool,
) -> SpuStepOutcome {
    let Some(scale) = scale(TO_FLOAT_SCALE_BIAS, imm) else {
        return SpuStepOutcome::Fault(SpuFault::UndefinedConversionScale(imm));
    };
    let a = words(state.regs[ra as usize]);
    let mut flags = [Flags::default(); 4];
    let results = std::array::from_fn(|slot| {
        let (negative, magnitude) = if signed {
            let value = a[slot] as i32;
            (value < 0, value.unsigned_abs())
        } else {
            (false, a[slot])
        };
        let exact = Exact::new(negative, u128::from(magnitude), -(scale as i32))
            .expect("invariant: a 32-bit integer fits the exact significand");
        let packed = round_pack::<Binary32>(Policy::SpuExtended, Rounding::TowardZero, exact);
        flags[slot] = packed.flags;
        packed.bits as u32
    });
    state.regs[rt as usize] = from_words(results);
    state.fpscr_accumulate_single(flags);
    SpuStepOutcome::Continue
}

/// `cflts` / `cfltu`: each slot's value times 2^scale, truncated toward
/// zero and saturated to the integer range.
// [SPU-ISA p:221 s:9] cflts saturates above 2^31 - 1 and below -2^31; [SPU-ISA p:223 s:9] cfltu saturates above 2^32 - 1 and every negative product to zero; [SPU-ISA p:196 s:9.1] truncation is the only single-precision rounding, and neither sets a flag.
pub(super) fn to_integer(
    state: &mut SpuState,
    rt: u8,
    ra: u8,
    imm: u8,
    signed: bool,
) -> SpuStepOutcome {
    let Some(scale) = scale(TO_INTEGER_SCALE_BIAS, imm) else {
        return SpuStepOutcome::Fault(SpuFault::UndefinedConversionScale(imm));
    };
    let a = words(state.regs[ra as usize]);
    state.regs[rt as usize] = from_words(a.map(|word| {
        let (x, _) = unpack_extended::<Binary32>(u64::from(word));
        // The magnitude truncated toward zero, or `None` at 2^33 and above.
        let shift = x.exponent() + scale as i32;
        let significand = x.significand();
        let magnitude = if significand == 0 {
            Some(0)
        } else if shift >= 0 {
            (128 - significand.leading_zeros() as i32 + shift <= 33).then(|| significand << shift)
        } else {
            Some(significand.checked_shr(shift.unsigned_abs()).unwrap_or(0))
        };
        match (signed, x.negative(), magnitude) {
            (true, false, Some(m)) if m <= i32::MAX as u128 => m as u32,
            (true, false, _) => i32::MAX as u32,
            (true, true, Some(m)) if m <= 1 << 31 => (m as u32).wrapping_neg(),
            (true, true, _) => i32::MIN as u32,
            (false, false, Some(m)) if m <= u128::from(u32::MAX) => m as u32,
            (false, false, _) => u32::MAX,
            (false, true, _) => 0,
        }
    }));
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
