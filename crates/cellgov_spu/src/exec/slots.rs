//! The per-slot helpers more than one instruction family uses.

use super::lanes::{from_halfwords, from_words, halfwords, words};
use super::outcome::SpuStepOutcome;
use crate::state::SpuState;

/// Applies `f` to each word slot of `ra` and `rb` and writes the four
/// results to `rt`.
///
/// It reads both sources before the write, so `rt` can alias `ra` or `rb`.
pub(super) fn words2(
    state: &mut SpuState,
    rt: u8,
    ra: u8,
    rb: u8,
    f: impl Fn(u32, u32) -> u32,
) -> SpuStepOutcome {
    let [a, b] = [ra, rb].map(|r| words(state.regs[r as usize]));
    state.set_reg(
        rt as usize,
        from_words(std::array::from_fn(|i| f(a[i], b[i]))),
    );
    SpuStepOutcome::Continue
}

/// The low halfword of a word, sign-extended.
pub(super) fn low_signed(word: u32) -> i32 {
    i32::from(word as u16 as i16)
}

/// Applies `f` to each halfword slot of `ra` and `rb` and writes the eight
/// results to `rt`, reading both sources before the write.
pub(super) fn halfwords2(
    state: &mut SpuState,
    rt: u8,
    ra: u8,
    rb: u8,
    f: impl Fn(u16, u16) -> u16,
) -> SpuStepOutcome {
    let [a, b] = [ra, rb].map(|r| halfwords(state.regs[r as usize]));
    state.set_reg(
        rt as usize,
        from_halfwords(std::array::from_fn(|i| f(a[i], b[i]))),
    );
    SpuStepOutcome::Continue
}
