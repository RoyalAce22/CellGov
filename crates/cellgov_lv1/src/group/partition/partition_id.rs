//! Hypercall 74, `get_logical_partition_id`: the id of the caller's
//! partition.
//!
//! CellGov runs the GameOS partition alone, so the answer is its id.

use cellgov_ps3_abi::lv1::call::GET_LOGICAL_PARTITION_ID;
use cellgov_ps3_abi::lv1::partition::LPAR_ID_GAME_OS;

use crate::answer::{HypercallAnswer, HypercallDisposition};
use crate::group::hypercall::Hypercall;

/// The hypercalls this module answers.
pub(crate) const HYPERCALLS: &[Hypercall] = &[Hypercall {
    number: GET_LOGICAL_PARTITION_ID,
    name: "get_logical_partition_id",
    cost: None,
    handler: |_, _| logical_partition_id(),
}];

/// The answer of `get_logical_partition_id`: `LV1_SUCCESS` and the
/// GameOS partition's id in r4.
fn logical_partition_id() -> HypercallDisposition {
    HypercallDisposition::Answered(HypercallAnswer::success(vec![LPAR_ID_GAME_OS]))
}
