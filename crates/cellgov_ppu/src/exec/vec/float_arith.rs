//! `vaddfp`, `vsubfp`, `vmaddfp` and `vnmsubfp`: each word slot through
//! the float engine.

use super::lanes::{build, element, map_u32};
use cellgov_float::{vmx_add, vmx_madd, vmx_nmsub, vmx_sub, VmxMode};

/// `vaddfp`: vA + vB in each slot.
pub(super) fn vaddfp(mode: VmxMode, a: u128, b: u128) -> u128 {
    map_u32(a, b, |x, y| vmx_add(mode, x, y))
}

/// `vsubfp`: vA - vB in each slot.
pub(super) fn vsubfp(mode: VmxMode, a: u128, b: u128) -> u128 {
    map_u32(a, b, |x, y| vmx_sub(mode, x, y))
}

/// `vmaddfp`: vA x vC + vB in each slot.
pub(super) fn vmaddfp(mode: VmxMode, a: u128, b: u128, c: u128) -> u128 {
    map3_u32(a, b, c, |x, y, z| vmx_madd(mode, x, y, z))
}

/// `vnmsubfp`: -(vA x vC - vB) in each slot.
pub(super) fn vnmsubfp(mode: VmxMode, a: u128, b: u128, c: u128) -> u128 {
    map3_u32(a, b, c, |x, y, z| vmx_nmsub(mode, x, y, z))
}

fn map3_u32(a: u128, b: u128, c: u128, f: impl Fn(u32, u32, u32) -> u32) -> u128 {
    let word = |v: u128, i: u32| element(v, 32, i) as u32;
    build(32, |i| u64::from(f(word(a, i), word(b, i), word(c, i))))
}
