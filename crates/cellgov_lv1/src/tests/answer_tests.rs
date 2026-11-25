//! The values an answer gives r4 to r10.
//!
//! lv1's return path loads r4 to r10 from return slots that the entry
//! `dcbz` [OFW-4.93 lv1 a:0x0C00-0x0C08] zeroed, so each slot that no
//! handler writes returns 0.

use cellgov_ps3_abi::lv1::status::{zero_extended, LV1_SUCCESS};

use super::HypercallAnswer;

#[test]
fn an_answer_puts_zero_in_each_register_after_its_outputs() {
    let answer = HypercallAnswer::lv1_status(5, [1, 2]);
    assert_eq!(answer.r3, 5);
    assert_eq!(answer.outputs, Some([1, 2, 0, 0, 0, 0, 0]));
}

#[test]
fn a_success_with_no_outputs_puts_zero_in_r4_to_r10() {
    let answer = HypercallAnswer::success([]);
    assert_eq!(answer.r3, zero_extended(LV1_SUCCESS));
    assert_eq!(answer.outputs, Some([0; 7]));
}

#[test]
fn an_r3_only_answer_gives_no_value_for_r4_to_r10() {
    assert_eq!(HypercallAnswer::r3_only(5).outputs, None);
}
