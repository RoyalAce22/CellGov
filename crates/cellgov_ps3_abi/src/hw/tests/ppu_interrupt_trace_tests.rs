use super::*;

#[test]
fn the_trace_vector_and_its_srr1_bit_are_the_handbook_values() {
    // [CBE-Handbook p:245 s:9.5] Table 9-3 places the trace interrupt at 0xD00.
    assert_eq!(VECTOR_TRACE, 0xD00);
    // [CBE-Handbook p:257 s:9.5.14] Table 9-16: a trace interrupt sets SRR1 bit 43 and clears bits 33:36, 42 and 44:47.
    assert_eq!(SRR1_TRACE, 0x0000_0000_0010_0000);
    assert_eq!(SRR1_TRACE & !SRR1_INTERRUPT_INFO, 0);
}
