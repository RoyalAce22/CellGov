//! `vaddfp` and `vsubfp`: each word slot through the float engine.

use super::lanes::map_u32;
use cellgov_float::{vmx_add, vmx_sub, VmxMode};

/// `vaddfp`: vA + vB in each slot.
pub(super) fn vaddfp(mode: VmxMode, a: u128, b: u128) -> u128 {
    map_u32(a, b, |x, y| vmx_add(mode, x, y))
}

/// `vsubfp`: vA - vB in each slot.
pub(super) fn vsubfp(mode: VmxMode, a: u128, b: u128) -> u128 {
    map_u32(a, b, |x, y| vmx_sub(mode, x, y))
}
