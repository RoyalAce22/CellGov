//! Bit select, byte permute and the double-vector byte shift.

use super::lanes::{build, element};

pub(super) fn vsel(a: u128, b: u128, c: u128) -> u128 {
    // [AltiVec-PEM p:6-133 s:6.2] per-bit mux: c_bit ? b_bit : a_bit.
    (a & !c) | (b & c)
}

/// Byte `k` of the 32-byte concatenation vA || vB.
fn joined_byte(a: u128, b: u128, k: u32) -> u64 {
    element(if k < 16 { a } else { b }, 8, k % 16)
}

pub(super) fn vperm(a: u128, b: u128, c: u128) -> u128 {
    // [AltiVec-PEM p:6-112 s:6.2] low 5 bits of each c byte index the
    // a:b concatenation (32 bytes).
    build(8, |i| joined_byte(a, b, element(c, 8, i) as u32 & 0x1F))
}

pub(super) fn vsldoi(a: u128, b: u128, sh: u8) -> u128 {
    // [AltiVec-PEM p:6-136 s:6.2] SH is 4-bit; left-shift the a:b
    // concatenation by `sh` bytes and return the high 16.
    build(8, |i| joined_byte(a, b, i + u32::from(sh & 0xF)))
}
