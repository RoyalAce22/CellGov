use super::*;

#[test]
fn the_fp_trace_and_exception_mode_msr_bits_sit_where_book3_numbers_them() {
    assert_eq!(MSR_FP, 0x2000);
    assert_eq!(MSR_FE0, 0x800);
    assert_eq!(MSR_SE, 0x400);
    assert_eq!(MSR_BE, 0x200);
    assert_eq!(MSR_FE1, 0x100);
}
