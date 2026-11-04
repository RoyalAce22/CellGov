//! The word a single-precision store writes: the ISA's bit selection,
//! with no rounding and no saturation.

use super::*;

#[test]
fn a_double_above_the_single_range_stores_the_selected_bits() {
    // [PPC-Book1 p:106 s:4.6.3] A value above the single range stores a well-defined word that loads back unequal.
    for (d, word) in [
        (0x7E37_E43C_8800_759C, 0x71BF_21E4), // 1e300
        (0xFE37_E43C_8800_759C, 0xF1BF_21E4), // -1e300
        (0x7FEF_FFFF_FFFF_FFFF, 0x7F7F_FFFF), // the largest double
    ] {
        assert_eq!(single_frs(d), word, "0x{d:016x}");
        assert_ne!(
            f64::from(f32::from_bits(word)).to_bits(),
            d,
            "0x{d:016x} loads back unequal"
        );
    }
}

#[test]
fn fraction_bits_below_single_precision_are_dropped() {
    // 1 + 2^-23 stores as 0x3F800001. FRS bits 35:63 lie below single
    // precision; bit 35 alone is the halfway point, where rounding to
    // nearest even would store 0x3F800002.
    let base = 0x3FF0_0000_2000_0000u64;
    for bit in 35..64 {
        let d = base | (1 << (63 - bit));
        assert_eq!(single_frs(d), 0x3F80_0001, "FRS bit {bit}");
    }
    assert_eq!(single_frs(base | 0x1FFF_FFFF), 0x3F80_0001, "all of 35:63");
}

#[test]
fn the_single_denormal_range_shifts_and_truncates() {
    // Biased exponent e in 874..=896 with an all-ones fraction keeps
    // e - 873 ones in the low bits of the word.
    for e in 874u64..=896 {
        let ones = (1u32 << (e - 873)) - 1;
        let d = (e << 52) | 0x000F_FFFF_FFFF_FFFF;
        assert_eq!(single_frs(d), ones, "exponent {e}");
        assert_eq!(
            single_frs(d | (1 << 63)),
            0x8000_0000 | ones,
            "-exponent {e}"
        );
    }
}

#[test]
fn zeros_infinities_and_nans_keep_their_bits() {
    for (d, word) in [
        (0x0000_0000_0000_0000, 0x0000_0000),
        (0x8000_0000_0000_0000, 0x8000_0000),
        (0x7FF0_0000_0000_0000, 0x7F80_0000),
        (0xFFF0_0000_0000_0000, 0xFF80_0000),
        (0x7FF8_0000_0000_0000, 0x7FC0_0000),
        (0x7FF0_0000_2000_0000, 0x7F80_0001),
    ] {
        assert_eq!(single_frs(d), word, "0x{d:016x}");
    }
}

/// [PPC-Book1 p:106 s:4.6.3] The book leaves the word undefined below biased exponent 874.
#[test]
fn a_double_below_the_denormal_range_stores_a_signed_zero() {
    assert_eq!(single_frs(873 << 52), 0);
    assert_eq!(single_frs((1 << 63) | 1), 0x8000_0000);
}
