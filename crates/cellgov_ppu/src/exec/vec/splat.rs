//! Splat an element or a sign-extended 5-bit immediate across a vector.

use super::lanes::{build, element};

/// Element `idx` modulo the element count of `b`, with `w`-bit
/// elements, in every element.
fn splat(b: u128, w: u32, idx: u8) -> u128 {
    let e = element(b, w, u32::from(idx) % (128 / w));
    build(w, |_| e)
}

/// The 5-bit immediate `imm`, sign-extended to `w` bits, in every
/// element.
fn splat_immediate(w: u32, imm: u8) -> u128 {
    let value = ((u64::from(imm) << 59) as i64 >> 59) as u64;
    build(w, |_| value)
}

pub(super) fn vspltb(b: u128, idx: u8) -> u128 {
    splat(b, 8, idx)
}

pub(super) fn vsplth(b: u128, idx: u8) -> u128 {
    splat(b, 16, idx)
}

pub(super) fn vspltw(b: u128, idx: u8) -> u128 {
    splat(b, 32, idx)
}

pub(super) fn vspltisb(imm: u8) -> u128 {
    splat_immediate(8, imm)
}

pub(super) fn vspltish(imm: u8) -> u128 {
    splat_immediate(16, imm)
}

pub(super) fn vspltisw(imm: u8) -> u128 {
    splat_immediate(32, imm)
}
