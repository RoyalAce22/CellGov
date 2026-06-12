//! Raw-word decoder dispatch: encoding recognition and named rejection.

// Several encoding tests below set bit-field slots to literal `0`
// via `(0u32 << N)` -- the explicit shift makes the field
// placement readable next to its sibling shifts, so the
// identity-op lint is silenced for the whole module rather
// than collapsing the documented zeros at each site.
#![allow(clippy::identity_op)]

use super::*;

#[path = "decode_branch_sc_tests.rs"]
mod branch_sc;
#[path = "decode_family_tests.rs"]
mod families;
#[path = "decode_gap_tests.rs"]
mod gaps;
#[path = "decode_round_trip_tests.rs"]
mod round_trip;
#[path = "decode_scalar_tests.rs"]
mod scalar;
#[path = "decode_spr_cr_tests.rs"]
mod spr_cr;
