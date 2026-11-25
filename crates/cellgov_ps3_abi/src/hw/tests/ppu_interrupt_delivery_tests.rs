use super::*;

#[test]
fn the_external_and_decrementer_vectors_are_the_figure_36_addresses() {
    assert_eq!(VECTOR_EXTERNAL, 0x500);
    assert_eq!(VECTOR_DECREMENTER, 0x900);
}

#[test]
fn the_srr1_interrupt_information_is_bits_33_to_36_and_42_to_47() {
    let bits = (33..=36)
        .chain(42..=47)
        .fold(0, |m, b| m | doubleword_bit(b));
    assert_eq!(SRR1_INTERRUPT_INFO, bits);
    for cause in [
        SRR1_FETCH_NOT_FOUND,
        SRR1_FETCH_NO_EXECUTE,
        SRR1_FETCH_PROTECTION,
        SRR1_PROGRAM_FP_ENABLED,
        SRR1_PROGRAM_ILLEGAL,
        SRR1_PROGRAM_PRIVILEGED,
        SRR1_PROGRAM_TRAP,
    ] {
        assert_eq!(cause & !SRR1_INTERRUPT_INFO, 0, "{cause:#x}");
    }
}
