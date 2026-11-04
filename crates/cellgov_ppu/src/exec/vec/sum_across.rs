//! Sums across the elements of vA into the words of vB, saturated.
//!
//! Each returns, beside the result, whether any word saturated.

use super::integer::{clamp_signed, clamp_unsigned};
use super::lanes::map_u32_sat;

/// The four words of `v`, word 0 first.
fn words(v: u128) -> [u32; 4] {
    std::array::from_fn(|i| (v >> (96 - 32 * i)) as u32)
}

/// Four words, word 0 first, as a vector.
fn pack(w: [u32; 4]) -> u128 {
    w.iter().fold(0u128, |v, &x| (v << 32) | u128::from(x))
}

fn signed(x: u32) -> i64 {
    i64::from(x as i32)
}

/// `vsumsws`: word 3 of vB plus the four words of vA, saturated to a
/// signed word in word 3 of vD; words 0..2 of vD are 0.
pub(super) fn vsumsws(a: u128, b: u128) -> (u128, bool) {
    let total = signed(words(b)[3]) + words(a).iter().map(|&x| signed(x)).sum::<i64>();
    let (r, s) = clamp_signed(total, 32);
    (pack([0, 0, 0, r as u32]), s)
}

/// `vsum2sws`: words 0 + 1 of vA plus word 1 of vB into word 1 of vD,
/// and words 2 + 3 plus word 3 into word 3, each saturated to a signed
/// word; words 0 and 2 of vD are 0.
pub(super) fn vsum2sws(a: u128, b: u128) -> (u128, bool) {
    let (wa, wb) = (words(a), words(b));
    let half = |k: usize| {
        clamp_signed(
            signed(wb[2 * k + 1]) + signed(wa[2 * k]) + signed(wa[2 * k + 1]),
            32,
        )
    };
    let ((lo, s0), (hi, s1)) = (half(0), half(1));
    (pack([0, lo as u32, 0, hi as u32]), s0 || s1)
}

/// `vsum4sbs`: each vB word plus the four signed bytes of the vA word,
/// saturated to a signed word.
pub(super) fn vsum4sbs(a: u128, b: u128) -> (u128, bool) {
    map_u32_sat(a, b, |x, y| {
        let bytes: i64 = x.to_be_bytes().iter().map(|&v| i64::from(v as i8)).sum();
        let (r, s) = clamp_signed(signed(y) + bytes, 32);
        (r as u32, s)
    })
}

/// `vsum4shs`: each vB word plus the two signed half words of the vA
/// word, saturated to a signed word.
pub(super) fn vsum4shs(a: u128, b: u128) -> (u128, bool) {
    map_u32_sat(a, b, |x, y| {
        let halfs = i64::from((x >> 16) as i16) + i64::from(x as i16);
        let (r, s) = clamp_signed(signed(y) + halfs, 32);
        (r as u32, s)
    })
}

/// `vsum4ubs`: each vB word plus the four unsigned bytes of the vA word,
/// saturated to an unsigned word.
pub(super) fn vsum4ubs(a: u128, b: u128) -> (u128, bool) {
    map_u32_sat(a, b, |x, y| {
        let bytes: i64 = x.to_be_bytes().iter().map(|&v| i64::from(v)).sum();
        let (r, s) = clamp_unsigned(i64::from(y) + bytes, 32);
        (r as u32, s)
    })
}
