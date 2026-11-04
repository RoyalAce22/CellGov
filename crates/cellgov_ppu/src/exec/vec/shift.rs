//! Per-element shifts. Each element's shift count is the low bits of
//! the matching element of vB.

use super::lanes::{map_u16, map_u32, map_u8};

pub(super) fn vslw(a: u128, b: u128) -> u128 {
    map_u32(a, b, |x, y| x << (y & 0x1F))
}

pub(super) fn vsrw(a: u128, b: u128) -> u128 {
    map_u32(a, b, |x, y| x >> (y & 0x1F))
}

pub(super) fn vsraw(a: u128, b: u128) -> u128 {
    map_u32(a, b, |x, y| ((x as i32) >> (y & 0x1F)) as u32)
}

pub(super) fn vsrah(a: u128, b: u128) -> u128 {
    map_u16(a, b, |x, y| ((x as i16) >> (y & 0xF)) as u16)
}

pub(super) fn vsrab(a: u128, b: u128) -> u128 {
    map_u8(a, b, |x, y| ((x as i8) >> (y & 0x7)) as u8)
}

pub(super) fn vslb(a: u128, b: u128) -> u128 {
    map_u8(a, b, |x, y| x << (y & 0x7))
}

pub(super) fn vslh(a: u128, b: u128) -> u128 {
    map_u16(a, b, |x, y| x << (y & 0xF))
}

pub(super) fn vsrb(a: u128, b: u128) -> u128 {
    map_u8(a, b, |x, y| x >> (y & 0x7))
}

pub(super) fn vsrh(a: u128, b: u128) -> u128 {
    map_u16(a, b, |x, y| x >> (y & 0xF))
}

pub(super) fn vrlb(a: u128, b: u128) -> u128 {
    map_u8(a, b, |x, y| x.rotate_left(u32::from(y & 0x7)))
}

pub(super) fn vrlh(a: u128, b: u128) -> u128 {
    map_u16(a, b, |x, y| x.rotate_left(u32::from(y & 0xF)))
}

pub(super) fn vrlw(a: u128, b: u128) -> u128 {
    map_u32(a, b, |x, y| x.rotate_left(y & 0x1F))
}
