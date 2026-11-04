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

pub(super) fn vcmpequb(a: u128, b: u128) -> u128 {
    map_u8(a, b, |x, y| if x == y { u8::MAX } else { 0 })
}

pub(super) fn vcmpequh(a: u128, b: u128) -> u128 {
    map_u16(a, b, |x, y| if x == y { u16::MAX } else { 0 })
}

pub(super) fn vcmpgtub(a: u128, b: u128) -> u128 {
    map_u8(a, b, |x, y| if x > y { u8::MAX } else { 0 })
}

pub(super) fn vcmpgtuh(a: u128, b: u128) -> u128 {
    map_u16(a, b, |x, y| if x > y { u16::MAX } else { 0 })
}

pub(super) fn vcmpgtuw(a: u128, b: u128) -> u128 {
    map_u32(a, b, |x, y| if x > y { u32::MAX } else { 0 })
}

pub(super) fn vcmpgtsb(a: u128, b: u128) -> u128 {
    map_u8(a, b, |x, y| if x as i8 > y as i8 { u8::MAX } else { 0 })
}

pub(super) fn vcmpgtsh(a: u128, b: u128) -> u128 {
    map_u16(a, b, |x, y| if x as i16 > y as i16 { u16::MAX } else { 0 })
}

pub(super) fn vcmpgtsw(a: u128, b: u128) -> u128 {
    map_u32(a, b, |x, y| if x as i32 > y as i32 { u32::MAX } else { 0 })
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

/// `vmuleuh`: the even halfword of each word of vA times that of vB,
/// as a word.
pub(super) fn vmuleuh(a: u128, b: u128) -> u128 {
    map_u32(a, b, |x, y| (x >> 16) * (y >> 16))
}

/// `vmulosh`: the odd halfwords, signed, as a word.
pub(super) fn vmulosh(a: u128, b: u128) -> u128 {
    map_u32(a, b, |x, y| {
        (i32::from(x as i16) * i32::from(y as i16)) as u32
    })
}

/// `vmulesh`: the even halfwords, signed, as a word.
pub(super) fn vmulesh(a: u128, b: u128) -> u128 {
    map_u32(a, b, |x, y| {
        (i32::from((x >> 16) as i16) * i32::from((y >> 16) as i16)) as u32
    })
}

/// `vmuloub`: the odd byte of each halfword of vA times that of vB, as
/// a halfword.
pub(super) fn vmuloub(a: u128, b: u128) -> u128 {
    map_u16(a, b, |x, y| (x & 0xFF) * (y & 0xFF))
}

/// `vmuleub`: the even byte of each halfword of vA times that of vB, as
/// a halfword.
pub(super) fn vmuleub(a: u128, b: u128) -> u128 {
    map_u16(a, b, |x, y| (x >> 8) * (y >> 8))
}

/// `vmulosb`: the odd bytes, signed, as a halfword.
pub(super) fn vmulosb(a: u128, b: u128) -> u128 {
    map_u16(a, b, |x, y| {
        (i16::from(x as i8) * i16::from(y as i8)) as u16
    })
}

/// `vmulesb`: the even bytes, signed, as a halfword.
pub(super) fn vmulesb(a: u128, b: u128) -> u128 {
    map_u16(a, b, |x, y| {
        (i16::from((x >> 8) as i8) * i16::from((y >> 8) as i8)) as u16
    })
}
