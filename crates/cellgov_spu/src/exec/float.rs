//! Single-precision arithmetic on the `cellgov_float` engine. For each
//! word slot the engine decodes the operands, combines them exactly, then
//! truncates and packs the result; the slot's flags OR into its FPSCR
//! bits. The arms hold no floating-point logic.

use cellgov_float::{round_pack, unpack_extended, Binary32, Exact, Flags, Policy, Rounding};

use super::lanes::{from_words, words};
use super::outcome::SpuStepOutcome;
use crate::state::SpuState;

/// Applies `op` to each word slot of `ra` and `rb` as single-precision
/// values, writes the packed results to `rt`, and ORs each slot's flags
/// into the FPSCR.
// [SPU-ISA p:196 s:9.1] fa, fs and fm set OVF, UNF and DIFF in the slice of each slot.
pub(super) fn single(
    state: &mut SpuState,
    rt: u8,
    ra: u8,
    rb: u8,
    op: impl Fn(Exact, Exact) -> Exact,
) -> SpuStepOutcome {
    let [a, b] = [ra, rb].map(|r| words(state.regs[r as usize]));
    let mut flags = [Flags::default(); 4];
    let results = std::array::from_fn(|slot| {
        let (x, x_flags) = unpack_extended::<Binary32>(u64::from(a[slot]));
        let (y, y_flags) = unpack_extended::<Binary32>(u64::from(b[slot]));
        // The extended policy truncates whatever mode is named.
        let packed = round_pack::<Binary32>(Policy::SpuExtended, Rounding::TowardZero, op(x, y));
        flags[slot] = x_flags.or(y_flags).or(packed.flags);
        packed.bits as u32
    });
    state.regs[rt as usize] = from_words(results);
    state.fpscr_accumulate_single(flags);
    SpuStepOutcome::Continue
}

/// `a + b` for two decoded single-precision operands.
pub(super) fn sum(a: Exact, b: Exact) -> Exact {
    cellgov_float::add(a, b)
        .expect("invariant: decoded single-precision operands are exact and 24 bits wide")
}

/// `a * b` for two decoded single-precision operands.
pub(super) fn product(a: Exact, b: Exact) -> Exact {
    cellgov_float::mul(a, b)
        .expect("invariant: decoded single-precision operands are exact and 24 bits wide")
}
