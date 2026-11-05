//! `double_word` against the single-to-double widening that IEEE 754
//! defines as exact, for every non-NaN single class.

use super::double_word;

/// [PPC-Book1 p:103 s:4.6.2] DOUBLE(WORD) of a normalized, denormalized, zero or infinite single is that single's exact value in double format.
#[test]
fn every_exponent_widens_exactly() {
    let fractions = [
        0,
        1,
        2,
        3,
        0x0000_00FF,
        0x0001_0000,
        0x0040_0000,
        0x0055_5555,
        0x007F_FFFE,
        0x007F_FFFF,
    ];
    let mut checked = 0;
    for exp in 0..0xFF_u32 {
        for frac in fractions {
            for sign in [0, 1u32 << 31] {
                let w = sign | exp << 23 | frac;
                let want = f64::from(f32::from_bits(w)).to_bits();
                assert_eq!(double_word(w), want, "word {w:#010x}");
                checked += 1;
            }
        }
    }
    assert_eq!(checked, 255 * 10 * 2);
}

/// [PPC-Book1 p:103 s:4.6.2] every denormalized single normalizes in double format.
#[test]
fn every_denormal_widens_exactly() {
    for frac in 1..=0x007F_FFFF_u32 {
        let want = f64::from(f32::from_bits(frac)).to_bits();
        assert_eq!(double_word(frac), want, "word {frac:#010x}");
    }
}

/// [PPC-Book1 p:103 s:4.6.2] an infinity widens to an infinity, and a NaN keeps WORD1:31 followed by 29 zeros, its quiet bit unchanged.
#[test]
fn infinities_and_nans_keep_their_bits() {
    assert_eq!(double_word(0x7F80_0000), 0x7FF0_0000_0000_0000);
    assert_eq!(double_word(0xFF80_0000), 0xFFF0_0000_0000_0000);
    assert_eq!(double_word(0x7F80_0001), 0x7FF0_0000_2000_0000);
    assert_eq!(double_word(0xFFC0_0001), 0xFFF8_0000_2000_0000);
}
