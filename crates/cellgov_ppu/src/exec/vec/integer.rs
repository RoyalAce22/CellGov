//! Integer add, subtract, multiply and compare.

use super::lanes::{map_u16, map_u32, map_u8, map_u8_sat};

pub(super) fn vadd_bytes(a: u128, b: u128) -> u128 {
    map_u8(a, b, u8::wrapping_add)
}

pub(super) fn vadd_halfs(a: u128, b: u128) -> u128 {
    map_u16(a, b, u16::wrapping_add)
}

pub(super) fn vadd_words(a: u128, b: u128) -> u128 {
    map_u32(a, b, u32::wrapping_add)
}

/// `vaddcuw`: the carry out of each unsigned word sum, as 0 or 1.
pub(super) fn vaddcuw(a: u128, b: u128) -> u128 {
    map_u32(a, b, |x, y| u32::from(x.overflowing_add(y).1))
}

pub(super) fn vsub_bytes(a: u128, b: u128) -> u128 {
    map_u8(a, b, u8::wrapping_sub)
}

pub(super) fn vsub_halfs(a: u128, b: u128) -> u128 {
    map_u16(a, b, u16::wrapping_sub)
}

pub(super) fn vsub_words(a: u128, b: u128) -> u128 {
    map_u32(a, b, u32::wrapping_sub)
}

/// `vsubcuw`: the complement of the borrow out of each unsigned word
/// difference: 1 when the vA word is not less than the vB word, else 0.
pub(super) fn vsubcuw(a: u128, b: u128) -> u128 {
    map_u32(a, b, |x, y| u32::from(x >= y))
}

pub(super) fn vcmpequw(a: u128, b: u128) -> u128 {
    map_u32(a, b, |x, y| if x == y { u32::MAX } else { 0 })
}

/// `vsububs`: each byte of vA minus vB, clamped at 0. The flag is set
/// when an element clamps.
///
/// [AltiVec-PEM p:6-161 s:6.2] vsububs clamps each byte of vA minus vB at 0 and sets SAT when it clamps.
/// [AltiVec-PEM p:4-4 s:4.2.1.1] A result equal to the bound without clamping is not saturation.
pub(super) fn vsub_ubytes_sat(a: u128, b: u128) -> (u128, bool) {
    map_u8_sat(a, b, |x, y| match x.checked_sub(y) {
        Some(v) => (v, false),
        None => (0, true),
    })
}

/// `vmulouh`: the odd halfword of each word of vA times that of vB,
/// as a word.
pub(super) fn vmulouh(a: u128, b: u128) -> u128 {
    map_u32(a, b, |x, y| (x & 0xFFFF) * (y & 0xFFFF))
}
