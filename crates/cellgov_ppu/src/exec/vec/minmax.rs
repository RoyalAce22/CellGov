//! Element-wise maximum, minimum and average.
//!
//! An average adds the two elements and 1 in one extra bit, then keeps
//! the high-order element-width bits, so it rounds toward +infinity.

use super::lanes::{map_u16, map_u32, map_u8};

pub(super) fn vmaxub(a: u128, b: u128) -> u128 {
    map_u8(a, b, u8::max)
}

pub(super) fn vmaxuh(a: u128, b: u128) -> u128 {
    map_u16(a, b, u16::max)
}

pub(super) fn vmaxuw(a: u128, b: u128) -> u128 {
    map_u32(a, b, u32::max)
}

pub(super) fn vmaxsb(a: u128, b: u128) -> u128 {
    map_u8(a, b, |x, y| (x as i8).max(y as i8) as u8)
}

pub(super) fn vmaxsh(a: u128, b: u128) -> u128 {
    map_u16(a, b, |x, y| (x as i16).max(y as i16) as u16)
}

pub(super) fn vmaxsw(a: u128, b: u128) -> u128 {
    map_u32(a, b, |x, y| (x as i32).max(y as i32) as u32)
}

pub(super) fn vminub(a: u128, b: u128) -> u128 {
    map_u8(a, b, u8::min)
}

pub(super) fn vminuh(a: u128, b: u128) -> u128 {
    map_u16(a, b, u16::min)
}

pub(super) fn vminuw(a: u128, b: u128) -> u128 {
    map_u32(a, b, u32::min)
}

pub(super) fn vminsb(a: u128, b: u128) -> u128 {
    map_u8(a, b, |x, y| (x as i8).min(y as i8) as u8)
}

pub(super) fn vminsh(a: u128, b: u128) -> u128 {
    map_u16(a, b, |x, y| (x as i16).min(y as i16) as u16)
}

pub(super) fn vminsw(a: u128, b: u128) -> u128 {
    map_u32(a, b, |x, y| (x as i32).min(y as i32) as u32)
}

pub(super) fn vavgub(a: u128, b: u128) -> u128 {
    map_u8(a, b, |x, y| ((u16::from(x) + u16::from(y) + 1) >> 1) as u8)
}

pub(super) fn vavguh(a: u128, b: u128) -> u128 {
    map_u16(a, b, |x, y| ((u32::from(x) + u32::from(y) + 1) >> 1) as u16)
}

pub(super) fn vavguw(a: u128, b: u128) -> u128 {
    map_u32(a, b, |x, y| ((u64::from(x) + u64::from(y) + 1) >> 1) as u32)
}

pub(super) fn vavgsb(a: u128, b: u128) -> u128 {
    map_u8(a, b, |x, y| {
        ((i16::from(x as i8) + i16::from(y as i8) + 1) >> 1) as u8
    })
}

pub(super) fn vavgsh(a: u128, b: u128) -> u128 {
    map_u16(a, b, |x, y| {
        ((i32::from(x as i16) + i32::from(y as i16) + 1) >> 1) as u16
    })
}

pub(super) fn vavgsw(a: u128, b: u128) -> u128 {
    map_u32(a, b, |x, y| {
        ((i64::from(x as i32) + i64::from(y as i32) + 1) >> 1) as u32
    })
}
