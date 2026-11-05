//! The deterministic generator of the documented PPU list: one function
//! per family of sets, each emitting every case of its sets in a fixed
//! order.

pub(super) mod draft;
pub(super) mod float;
pub(super) mod integer;
pub(super) mod storage;
pub(super) mod vmx;

use super::vector::DocumentedVector;

/// The whole list, set by set in [`super::Set::ALL`] order.
pub fn documented_vectors() -> Vec<DocumentedVector> {
    [
        integer::muldiv_undef(),
        integer::move_undef(),
        float::fctiw_high_word(),
        float::mffs_high_word(),
        float::fp59_wide_operand(),
        float::fp_overflow_fr(),
        float::stfs_below_denormal(),
        float::fpscr_ni(),
        storage::stwcx_other_address(),
        storage::reservation_undefined(),
        vmx::lve_other_lanes(),
        vmx::vshift_nonuniform(),
    ]
    .concat()
}
