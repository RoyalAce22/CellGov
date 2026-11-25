use super::*;
use crate::hw::ppu_interrupt::VECTOR_EXTERNAL;
use crate::hw::ppu_supervisor::{MSR_EE, MSR_ME, MSR_SF};

/// lv1's raise keeps bits 0 to 47 and clears bits 48 to 63
/// [OFW-4.93 lv1 a:0x1904]. The first MSR is the GameOS kernel's MSR at
/// its first dispatcher manager write.
#[test]
fn the_raise_msr_clears_bits_48_to_63_and_keeps_the_rest() {
    assert_eq!(
        external_interrupt_msr(0x8000_0000_0000_8032),
        0x8000_0000_0000_0000
    );
    assert_eq!(
        external_interrupt_msr(0x9000_0001_0000_F0FF),
        0x9000_0001_0000_0000
    );
    assert_eq!(external_interrupt_msr(MSR_EE | MSR_ME), MSR_SF);
}

/// The raise sets SF, bit 0 [OFW-4.93 lv1 a:0x1908-0x1910].
#[test]
fn the_raise_msr_sets_sf() {
    assert_eq!(external_interrupt_msr(0), MSR_SF);
    assert_eq!(MSR_SF, 0x8000_0000_0000_0000);
}

#[test]
fn the_cleared_bits_are_msr_bits_48_to_63() {
    assert_eq!(EXTERNAL_INTERRUPT_MSR_CLEARED, (1 << 16) - 1);
    assert_eq!(EXTERNAL_INTERRUPT_MSR_CLEARED & MSR_EE, MSR_EE);
}

/// [CBE-Handbook p:306 s:10.4.3] LPCR bit 52 is MER.
#[test]
fn mer_is_lpcr_bit_52() {
    assert_eq!(LPCR_MER, 1 << (63 - 52));
}

/// The context constructor stores 0x500 at +496 [OFW-4.93 lv1 a:0x2C3DB8].
#[test]
fn the_vector_is_the_architected_external_interrupt_vector() {
    assert_eq!(EXTERNAL_INTERRUPT_VECTOR, 0x500);
    assert_eq!(EXTERNAL_INTERRUPT_VECTOR, VECTOR_EXTERNAL);
}
