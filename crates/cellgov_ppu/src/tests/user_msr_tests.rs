//! The state's mode-dependent results follow the declared user MSR:
//! 64-bit mode.

use super::*;

#[test]
fn an_effective_address_keeps_its_high_word() {
    let mut s = PpuState::new();
    s.set_gpr(1, 0x1_0000_0000);
    s.set_gpr(2, 0x10);
    assert_eq!(s.ea_d_form(1, 8), 0x1_0000_0008);
    assert_eq!(s.ea_x_form(1, 2), 0x1_0000_0010);
}

#[test]
fn cr0_compares_all_64_bits_of_the_result() {
    let mut s = PpuState::new();
    s.set_cr0_from_result(0x8000_0000);
    assert_eq!(s.cr_field(0), 0b0100, "positive as a doubleword");
    s.set_cr0_from_result(0x1_0000_0000);
    assert_eq!(s.cr_field(0), 0b0100, "nonzero above bit 32");
}
