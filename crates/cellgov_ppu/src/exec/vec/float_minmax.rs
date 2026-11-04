//! `vmaxfp` and `vminfp`: the larger or smaller of each pair of word
//! slots, ordered through the float engine.

use super::lanes::map_u32;
use cellgov_float::{vmx_compare, vmx_flush, vmx_nan, VmxMode};
use std::cmp::Ordering;

/// Each slot: the engine's NaN result when either word is a NaN, else
/// the word `pick` chooses, as `mode` reads it; two zeros take the sign
/// `zeros` gives from their two signs.
fn select(mode: VmxMode, a: u128, b: u128, pick: Ordering, zeros: fn(bool, bool) -> bool) -> u128 {
    map_u32(a, b, |x, y| {
        if let Some(nan) = vmx_nan(&[x, y]) {
            return nan;
        }
        let (x, y) = (vmx_flush(mode, x), vmx_flush(mode, y));
        if (x | y) & 0x7FFF_FFFF == 0 {
            return u32::from(zeros(x >> 31 == 1, y >> 31 == 1)) << 31;
        }
        match vmx_compare(mode, x, y) {
            Some(order) if order == pick.reverse() => y,
            _ => x,
        }
    })
}

/// `vmaxfp`: the larger slot; +0 is larger than -0.
pub(super) fn vmaxfp(mode: VmxMode, a: u128, b: u128) -> u128 {
    select(mode, a, b, Ordering::Greater, |x, y| x && y)
}

/// `vminfp`: the smaller slot; -0 is smaller than +0.
pub(super) fn vminfp(mode: VmxMode, a: u128, b: u128) -> u128 {
    select(mode, a, b, Ordering::Less, |x, y| x || y)
}
