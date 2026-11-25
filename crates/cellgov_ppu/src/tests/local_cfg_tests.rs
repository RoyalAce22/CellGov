//! [PPC-Book1 p:24 s:2.4] With AA = 1 a `b` or `bc` reaches EXTS(LI || 0b00) or EXTS(BD || 0b00) in 64-bit mode.

use super::*;

/// The lowest address that a negative I-form displacement reaches.
const TOP_32_MIB: u64 = 0xFFFF_FFFF_FE00_0000;

/// The lowest address that a negative B-form displacement reaches.
const TOP_32_KIB: u64 = 0xFFFF_FFFF_FFFF_8000;

#[test]
fn absolute_branch_with_the_most_negative_displacement_reaches_word_zero() {
    assert_eq!(
        local_branch_index(TOP_32_MIB, 3, -0x0200_0000, true, 8),
        Some(0)
    );
}

#[test]
fn absolute_branch_with_a_negative_displacement_reaches_its_word() {
    assert_eq!(
        local_branch_index(TOP_32_MIB, 3, -0x0200_0000 + 0x14, true, 8),
        Some(5)
    );
}

#[test]
fn absolute_conditional_branch_with_a_negative_displacement_reaches_its_word() {
    assert_eq!(
        local_branch_index(TOP_32_KIB, 0, -0x8000 + 0x10, true, 8),
        Some(4)
    );
}
