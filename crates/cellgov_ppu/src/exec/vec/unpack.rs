//! Widen the high or low half of vB to elements of twice the width.

use super::lanes::{map_u16, map_u32};

/// The `w`-bit elements of `half`, in order, each in the low bits of a
/// `2w`-bit element of the result.
fn spread(half: u64, w: u32) -> u128 {
    let mask = (1u64 << w) - 1;
    (0..64 / w).fold(0u128, |r, j| {
        let element = (half >> (64 - w * (j + 1))) & mask;
        r | (u128::from(element) << (128 - 2 * w * (j + 1)))
    })
}

fn high(b: u128) -> u64 {
    (b >> 64) as u64
}

fn low(b: u128) -> u64 {
    b as u64
}

fn signed_bytes(half: u64) -> u128 {
    let s = spread(half, 8);
    map_u16(s, s, |x, _| i16::from(x as u8 as i8) as u16)
}

fn signed_halfs(half: u64) -> u128 {
    let s = spread(half, 16);
    map_u32(s, s, |x, _| i32::from(x as u16 as i16) as u32)
}

/// A 1:5:5:5 pixel as 8:8:8:8: bit 0 sign-extended, each 5-bit channel
/// zero-extended.
fn pixels(half: u64) -> u128 {
    let s = spread(half, 16);
    map_u32(s, s, |x, _| {
        let alpha = if x & 0x8000 != 0 { 0xFF00_0000 } else { 0 };
        alpha | ((x >> 10) & 0x1F) << 16 | ((x >> 5) & 0x1F) << 8 | (x & 0x1F)
    })
}

pub(super) fn vupkhsb(b: u128) -> u128 {
    signed_bytes(high(b))
}

pub(super) fn vupklsb(b: u128) -> u128 {
    signed_bytes(low(b))
}

pub(super) fn vupkhsh(b: u128) -> u128 {
    signed_halfs(high(b))
}

pub(super) fn vupklsh(b: u128) -> u128 {
    signed_halfs(low(b))
}

pub(super) fn vupkhpx(b: u128) -> u128 {
    pixels(high(b))
}

pub(super) fn vupklpx(b: u128) -> u128 {
    pixels(low(b))
}
