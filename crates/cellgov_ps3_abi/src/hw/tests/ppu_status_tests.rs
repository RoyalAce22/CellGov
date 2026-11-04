use super::*;

#[test]
fn the_masks_sit_at_the_bits_the_isa_numbers() {
    assert_eq!(CR0_LT_GT_EQ, 0xe000_0000);
    assert_eq!(CR0_SO, 0x1000_0000);
    assert_eq!(XER_RESERVED, 0xffff_ffff_1fff_ff80);
    assert_eq!(FPSCR_FR, 0x0004_0000);
    assert_eq!(FPSCR_FPRF, 0x0001_f000);
    assert_eq!(FPSCR_VXCVI, 0x0000_0100);
    assert_eq!(FPSCR_NI, 0x0000_0004);
}
