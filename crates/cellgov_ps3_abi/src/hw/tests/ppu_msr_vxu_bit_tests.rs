use super::*;

#[test]
fn the_vxu_msr_bit_sits_at_bit_38_and_no_interrupt_clears_it() {
    // [CBE-Handbook p:304 s:10.4.1] The PPE's MSR holds VXU at bit 38.
    assert_eq!(MSR_VXU, 0x0000_0000_0200_0000);
    // [CBE-Handbook p:292 s:9.12] Table 9-40 lists the bits an interrupt clears, and VXU is not one of them.
    assert_eq!(MSR_INTERRUPT_CLEARED & MSR_VXU, 0);
}
