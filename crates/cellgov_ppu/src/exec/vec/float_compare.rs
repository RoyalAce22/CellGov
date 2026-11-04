//! The four single-precision vector compares. Each orders the word slots
//! through the float engine, which owns the NaN and denormal rules.

use super::lanes::map_u32;
use cellgov_float::{vmx_compare, VmxMode};
use std::cmp::Ordering;

/// All ones in each slot where the order of vA against vB satisfies
/// `holds`, else 0; a NaN in either slot gives 0.
fn compare(mode: VmxMode, a: u128, b: u128, holds: fn(Ordering) -> bool) -> u128 {
    map_u32(a, b, |x, y| {
        if vmx_compare(mode, x, y).is_some_and(holds) {
            u32::MAX
        } else {
            0
        }
    })
}

pub(super) fn vcmpeqfp(mode: VmxMode, a: u128, b: u128) -> u128 {
    compare(mode, a, b, Ordering::is_eq)
}

pub(super) fn vcmpgefp(mode: VmxMode, a: u128, b: u128) -> u128 {
    compare(mode, a, b, Ordering::is_ge)
}

pub(super) fn vcmpgtfp(mode: VmxMode, a: u128, b: u128) -> u128 {
    compare(mode, a, b, Ordering::is_gt)
}

/// `vcmpbfp`: bit 0 of each slot is set when vA > vB or unordered, bit 1
/// when vA < -vB or unordered.
pub(super) fn vcmpbfp(mode: VmxMode, a: u128, b: u128) -> u128 {
    map_u32(a, b, |x, y| {
        let le = vmx_compare(mode, x, y).is_some_and(Ordering::is_le);
        let ge = vmx_compare(mode, x, y ^ 0x8000_0000).is_some_and(Ordering::is_ge);
        u32::from(!le) << 31 | u32::from(!ge) << 30
    })
}
