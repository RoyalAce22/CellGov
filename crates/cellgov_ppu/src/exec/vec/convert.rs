//! Conversions between fixed-point words and single-precision words, each
//! slot through the float engine. Only vB is read; the unused second
//! operand of each map is 0.

use super::lanes::{map_u32, map_u32_sat};
use cellgov_float::{vmx_from_integer, vmx_to_signed, vmx_to_unsigned, VmxMode};

/// `vcfsx`: each signed word of vB / 2^`uimm`.
pub(super) fn vcfsx(b: u128, uimm: u8) -> u128 {
    map_u32(b, 0, |x, _| {
        vmx_from_integer(i64::from(x as i32), u32::from(uimm))
    })
}

/// `vcfux`: each unsigned word of vB / 2^`uimm`.
pub(super) fn vcfux(b: u128, uimm: u8) -> u128 {
    map_u32(b, 0, |x, _| vmx_from_integer(i64::from(x), u32::from(uimm)))
}

/// `vctsxs`: each slot of vB x 2^`uimm` as a saturated signed word, and
/// whether any slot saturated.
pub(super) fn vctsxs(mode: VmxMode, b: u128, uimm: u8) -> (u128, bool) {
    map_u32_sat(b, 0, |x, _| vmx_to_signed(mode, x, u32::from(uimm)))
}

/// `vctuxs`: each slot of vB x 2^`uimm` as a saturated unsigned word, and
/// whether any slot saturated.
pub(super) fn vctuxs(mode: VmxMode, b: u128, uimm: u8) -> (u128, bool) {
    map_u32_sat(b, 0, |x, _| vmx_to_unsigned(mode, x, u32::from(uimm)))
}
