use super::super::status::{zero_extended, LV1_NOT_IMPLEMENTED, LV1_NO_PRIVILEGE};
use super::{PROBLEM_STATE_DENIED_R3, UNKNOWN_HYPERCALL_R3};

#[test]
fn the_unknown_hypercall_r3_is_not_implemented_zero_extended() {
    assert_eq!(UNKNOWN_HYPERCALL_R3, zero_extended(LV1_NOT_IMPLEMENTED));
    assert_eq!(UNKNOWN_HYPERCALL_R3, 0x0000_0000_FFFF_FFEC);
}

#[test]
fn the_problem_state_denied_r3_is_no_privilege_zero_extended() {
    assert_eq!(PROBLEM_STATE_DENIED_R3, zero_extended(LV1_NO_PRIVILEGE));
    assert_eq!(PROBLEM_STATE_DENIED_R3, 0x0000_0000_FFFF_FFFD);
}

#[test]
fn zero_extension_keeps_the_low_word_and_clears_the_high_word() {
    assert_eq!(zero_extended(-1), 0x0000_0000_FFFF_FFFF);
    assert_eq!(zero_extended(0), 0);
    assert_eq!(zero_extended(-32768), 0x0000_0000_FFFF_8000);
}
