//! `vrefp`, `vrsqrtefp`, `vexptefp` and `vlogefp`, which pass each vB
//! slot through the PPE's estimate tables in the float engine.

use super::lanes::map_u32;
use cellgov_float::{
    vmx_exp2_estimate, vmx_log2_estimate, vmx_reciprocal_estimate, vmx_rsqrt_estimate, VmxMode,
};

/// `vrefp`: the estimate of 1 / vB in each slot.
pub(super) fn vrefp(mode: VmxMode, b: u128) -> u128 {
    map_u32(b, 0, |x, _| vmx_reciprocal_estimate(mode, x))
}

/// `vrsqrtefp`: the estimate of 1 / sqrt(vB) in each slot.
pub(super) fn vrsqrtefp(mode: VmxMode, b: u128) -> u128 {
    map_u32(b, 0, |x, _| vmx_rsqrt_estimate(mode, x))
}

/// `vexptefp`: the estimate of 2^vB in each slot.
pub(super) fn vexptefp(mode: VmxMode, b: u128) -> u128 {
    map_u32(b, 0, |x, _| vmx_exp2_estimate(mode, x))
}

/// `vlogefp`: the estimate of log2(vB) in each slot.
pub(super) fn vlogefp(mode: VmxMode, b: u128) -> u128 {
    map_u32(b, 0, |x, _| vmx_log2_estimate(mode, x))
}
