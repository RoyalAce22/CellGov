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

/// The octet count of `vslo` and `vsro`: bits 121:124 of vB.
fn octets(b: u128) -> u32 {
    ((b >> 3) & 0xF) as u32
}

pub(super) fn vslo(a: u128, b: u128) -> u128 {
    a << (8 * octets(b))
}

pub(super) fn vsro(a: u128, b: u128) -> u128 {
    a >> (8 * octets(b))
}

/// `vsl`: each byte of vA shifts left by the low three bits of the
/// matching byte of vB and takes its low bits from the next byte.
pub(super) fn vsl(a: u128, b: u128) -> u128 {
    let (a, b) = (a.to_be_bytes(), b.to_be_bytes());
    u128::from_be_bytes(std::array::from_fn(|i| {
        let next = a.get(i + 1).copied().unwrap_or(0);
        let pair = u16::from(a[i]) << 8 | u16::from(next);
        (pair << (b[i] & 7) >> 8) as u8
    }))
}

/// `vsr`: each byte of vA shifts right by the low three bits of the
/// matching byte of vB and takes its high bits from the previous byte.
pub(super) fn vsr(a: u128, b: u128) -> u128 {
    let (a, b) = (a.to_be_bytes(), b.to_be_bytes());
    u128::from_be_bytes(std::array::from_fn(|i| {
        let prev = if i == 0 { 0 } else { a[i - 1] };
        let pair = u16::from(prev) << 8 | u16::from(a[i]);
        (pair >> (b[i] & 7)) as u8
    }))
}
