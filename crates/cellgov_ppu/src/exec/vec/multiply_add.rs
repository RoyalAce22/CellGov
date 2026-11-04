//! Integer multiply-add and multiply-sum over three source vectors.
//!
//! Each returns, beside the result, whether any element saturated; the
//! modulo forms always return false.

use super::integer::{clamp_signed, clamp_unsigned};
use super::lanes::{map3_u16_sat, map3_u32_sat};

/// Bits 0:16 of the 32-bit product of two signed half words, plus
/// `round`, plus `c`, clamped to a signed half word.
fn mhadd(x: u16, y: u16, z: u16, round: i32) -> (u16, bool) {
    let prod = i32::from(x as i16) * i32::from(y as i16) + round;
    let (r, s) = clamp_signed(i64::from(prod >> 15) + i64::from(z as i16), 16);
    (r as u16, s)
}

/// `vmhaddshs`.
pub(super) fn vmhaddshs(a: u128, b: u128, c: u128) -> (u128, bool) {
    map3_u16_sat(a, b, c, |x, y, z| mhadd(x, y, z, 0))
}

/// `vmhraddshs`: as `vmhaddshs`, with 0x4000 added to the product first.
pub(super) fn vmhraddshs(a: u128, b: u128, c: u128) -> (u128, bool) {
    map3_u16_sat(a, b, c, |x, y, z| mhadd(x, y, z, 0x4000))
}

/// `vmladduhm`: the low half word of each product plus `c`.
pub(super) fn vmladduhm(a: u128, b: u128, c: u128) -> (u128, bool) {
    map3_u16_sat(a, b, c, |x, y, z| {
        ((u32::from(x) * u32::from(y) + u32::from(z)) as u16, false)
    })
}

/// The four byte products of a word of vA and vB; `signed_a` reads the
/// vA bytes signed.
fn byte_products(x: u32, y: u32, signed_a: bool) -> [i64; 4] {
    std::array::from_fn(|j| {
        let shift = 24 - 8 * j;
        let a = (x >> shift) as u8;
        let a = if signed_a {
            i64::from(a as i8)
        } else {
            i64::from(a)
        };
        a * i64::from((y >> shift) as u8)
    })
}

/// The two half-word products of a word of vA and vB; `signed` reads
/// both signed.
fn half_products(x: u32, y: u32, signed: bool) -> [i64; 2] {
    std::array::from_fn(|j| {
        let shift = 16 - 16 * j;
        let (a, b) = ((x >> shift) as u16, (y >> shift) as u16);
        if signed {
            i64::from(a as i16) * i64::from(b as i16)
        } else {
            i64::from(a) * i64::from(b)
        }
    })
}

/// `vmsumubm`: each word of `c` plus the four unsigned byte products,
/// modulo 2^32.
pub(super) fn vmsumubm(a: u128, b: u128, c: u128) -> (u128, bool) {
    map3_u32_sat(a, b, c, |x, y, z| {
        let sum: i64 = byte_products(x, y, false).iter().sum();
        ((i64::from(z) + sum) as u32, false)
    })
}

/// `vmsummbm`: each word of `c` plus the four signed-by-unsigned byte
/// products, modulo 2^32.
pub(super) fn vmsummbm(a: u128, b: u128, c: u128) -> (u128, bool) {
    map3_u32_sat(a, b, c, |x, y, z| {
        let sum: i64 = byte_products(x, y, true).iter().sum();
        ((i64::from(z) + sum) as u32, false)
    })
}

/// `vmsumuhm`: each word of `c` plus the two unsigned half-word
/// products, modulo 2^32.
pub(super) fn vmsumuhm(a: u128, b: u128, c: u128) -> (u128, bool) {
    map3_u32_sat(a, b, c, |x, y, z| {
        let sum: i64 = half_products(x, y, false).iter().sum();
        ((i64::from(z) + sum) as u32, false)
    })
}

/// `vmsumuhs`: as `vmsumuhm`, clamped to an unsigned word.
pub(super) fn vmsumuhs(a: u128, b: u128, c: u128) -> (u128, bool) {
    map3_u32_sat(a, b, c, |x, y, z| {
        let sum: i64 = half_products(x, y, false).iter().sum();
        let (r, s) = clamp_unsigned(i64::from(z) + sum, 32);
        (r as u32, s)
    })
}

/// `vmsumshm`: each word of `c` plus the two signed half-word products,
/// modulo 2^32.
pub(super) fn vmsumshm(a: u128, b: u128, c: u128) -> (u128, bool) {
    map3_u32_sat(a, b, c, |x, y, z| {
        let sum: i64 = half_products(x, y, true).iter().sum();
        ((i64::from(z as i32) + sum) as u32, false)
    })
}

/// `vmsumshs`: as `vmsumshm`, clamped to a signed word.
pub(super) fn vmsumshs(a: u128, b: u128, c: u128) -> (u128, bool) {
    map3_u32_sat(a, b, c, |x, y, z| {
        let sum: i64 = half_products(x, y, true).iter().sum();
        let (r, s) = clamp_signed(i64::from(z as i32) + sum, 32);
        (r as u32, s)
    })
}
