use super::*;

#[test]
fn the_masks_sit_at_the_bits_the_isa_numbers() {
    assert_eq!(SRR1_FETCH_NOT_FOUND, 0x0000_0000_4000_0000);
    assert_eq!(SRR1_FETCH_NO_EXECUTE, 0x0000_0000_1000_0000);
    assert_eq!(SRR1_FETCH_PROTECTION, 0x0000_0000_0800_0000);
    assert_eq!(SRR1_PROGRAM_FP_ENABLED, 0x0000_0000_0010_0000);
    assert_eq!(SRR1_PROGRAM_ILLEGAL, 0x0000_0000_0008_0000);
    assert_eq!(SRR1_PROGRAM_PRIVILEGED, 0x0000_0000_0004_0000);
    assert_eq!(SRR1_PROGRAM_TRAP, 0x0000_0000_0002_0000);
    assert_eq!(DSISR_NOT_FOUND, 0x4000_0000);
    assert_eq!(DSISR_PROTECTION, 0x0800_0000);
    assert_eq!(DSISR_STORE, 0x0200_0000);
    assert_eq!(DSISR_SEGMENT, 0x0020_0000);
}
