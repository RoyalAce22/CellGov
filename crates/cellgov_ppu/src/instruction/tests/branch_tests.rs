use super::branch_target;

#[test]
fn a_relative_branch_adds_its_signed_displacement_to_its_own_address() {
    assert_eq!(branch_target(0x1_0000, 0x80, false), 0x1_0080);
    assert_eq!(branch_target(0x1_0000, -0x80, false), 0xFF80);
    assert_eq!(
        branch_target(0, -4, false),
        0xFFFF_FFFF_FFFF_FFFC,
        "the sum wraps"
    );
}

#[test]
fn an_absolute_branch_takes_the_sign_extended_displacement() {
    assert_eq!(branch_target(0x1_0000, 0x100, true), 0x100);
    assert_eq!(branch_target(0x1_0000, -0x100, true), 0xFFFF_FFFF_FFFF_FF00);
}
