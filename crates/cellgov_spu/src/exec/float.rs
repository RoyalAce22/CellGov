//! Single-precision arithmetic on the `cellgov_float` engine. For each
//! word slot the engine decodes the operands, combines them exactly, then
//! truncates and packs the result; the slot's flags OR into its FPSCR
//! bits. The arms hold no floating-point logic.

use cellgov_float::{round_pack, unpack_extended, Binary32, Exact, Flags, Policy, Rounding};

use super::lanes::{from_words, words};
use super::outcome::SpuStepOutcome;
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
