//! `vmx_from_integer`, `vmx_to_signed` and `vmx_to_unsigned` against host
//! float references, then fixed vectors at the rounding and saturation
//! boundaries.

use crate::vmx::{vmx_from_integer, vmx_to_signed, vmx_to_unsigned, VmxMode};

const MODES: [VmxMode; 2] = [VmxMode::Java, VmxMode::NonJava];

/// `value` / 2^`scale` from the host: one round to nearest even in the
/// `as f32`, then an exact division by a power of two.
fn from_reference(value: i64, scale: u32) -> u32 {
    (value as f32 / (1u64 << scale) as f32).to_bits()
}

/// `b` x 2^`scale` truncated, as the host computes it in binary64, where
/// every product is exact. `None` for a NaN.
fn scaled_reference(mode: VmxMode, b: u32, scale: u32) -> Option<f64> {
    let x = f32::from_bits(b);
    if x.is_nan() {
        return None;
    }
    let denormal = b & 0x7F80_0000 == 0 && b & 0x7F_FFFF != 0;
    if mode == VmxMode::NonJava && denormal {
        return Some(0.0);
    }
    Some((f64::from(x) * (1u64 << scale) as f64).trunc())
}

fn to_signed_reference(mode: VmxMode, b: u32, scale: u32) -> (u32, bool) {
    let Some(v) = scaled_reference(mode, b, scale) else {
        return (0, false);
    };
    if v > f64::from(i32::MAX) {
        (0x7FFF_FFFF, true)
    } else if v < f64::from(i32::MIN) {
        (0x8000_0000, true)
    } else {
        (v as i32 as u32, false)
    }
}

fn to_unsigned_reference(mode: VmxMode, b: u32, scale: u32) -> (u32, bool) {
    let Some(v) = scaled_reference(mode, b, scale) else {
        return (0, false);
    };
    if v > f64::from(u32::MAX) {
        (0xFFFF_FFFF, true)
    } else if v < 0.0 {
        (0, true)
    } else {
        (v as u32, false)
    }
}

/// Integer words around each rounding boundary of binary32.
fn integers() -> Vec<u32> {
    let mut words = vec![0, 1, 2, 3, 0x7FFF_FFFF, 0x8000_0000, 0xFFFF_FFFF];
    for top in [23u32, 24, 25, 26, 30, 31] {
        let base = 1u32 << top;
        for d in [0u32, 1, 2, 3, 0x7F, 0x80, 0x81, 0xFF] {
            words.extend([
                base.wrapping_add(d),
                base.wrapping_sub(d),
                !base.wrapping_add(d),
            ]);
        }
    }
    words
}

/// Binary32 words whose products cross every word's range at some scale.
///
/// - the denormals and values below one
/// - fractions of both signs
/// - values near 2^31 and 2^32
/// - the largest finite value, the infinities and two NaNs
fn floats() -> Vec<u32> {
    let exponents = [
        0u32, 1, 96, 120, 126, 127, 128, 150, 156, 157, 158, 159, 160, 254,
    ];
    let fractions = [0u32, 1, 0x40_0000, 0x3F_FFFF, 0x7F_FFFF];
    let mut words: Vec<u32> = exponents
        .iter()
        .flat_map(|&e| fractions.iter().map(move |&f| e << 23 | f))
        .chain([0x7F80_0000, 0x7FC0_0001, 0x7F80_0001])
        .collect();
    words.extend(words.clone().iter().map(|w| w | 0x8000_0000));
    words
}

#[test]
fn every_integer_converts_like_the_host_at_every_scale() {
    let words = integers();
    assert_eq!(words.len(), 7 + 6 * 8 * 3);
    for scale in 0..32 {
        for &w in &words {
            let signed = i64::from(w as i32);
            let unsigned = i64::from(w);
            assert_eq!(
                vmx_from_integer(signed, scale),
                from_reference(signed, scale),
                "vcfsx {w:#010x} / 2^{scale}"
            );
            assert_eq!(
                vmx_from_integer(unsigned, scale),
                from_reference(unsigned, scale),
                "vcfux {w:#010x} / 2^{scale}"
            );
        }
    }
}

#[test]
fn every_float_converts_like_the_host_at_every_scale() {
    let words = floats();
    assert_eq!(words.len(), 2 * (14 * 5 + 3));
    for mode in MODES {
        for scale in 0..32 {
            for &b in &words {
                assert_eq!(
                    vmx_to_signed(mode, b, scale),
                    to_signed_reference(mode, b, scale),
                    "vctsxs {mode:?} {b:#010x} x 2^{scale}"
                );
                assert_eq!(
                    vmx_to_unsigned(mode, b, scale),
                    to_unsigned_reference(mode, b, scale),
                    "vctuxs {mode:?} {b:#010x} x 2^{scale}"
                );
            }
        }
    }
}

/// [AltiVec-PEM p:6-4 s:6.1.2] ties go to even.
#[test]
fn integer_conversions_round_ties_to_even() {
    let signed = |w: u32, s| vmx_from_integer(i64::from(w as i32), s);
    let unsigned = |w: u32, s| vmx_from_integer(i64::from(w), s);
    assert_eq!(signed(0x0100_0001, 0), 0x4B80_0000);
    assert_eq!(signed(0x0100_0003, 0), 0x4B80_0002);
    assert_eq!(signed(0x7FFF_FFFF, 0), 0x4F00_0000);
    assert_eq!(signed(0x8000_0000, 0), 0xCF00_0000);
    assert_eq!(unsigned(0xFFFF_FFFF, 0), 0x4F80_0000);
    assert_eq!(unsigned(0xFFFF_FF80, 0), 0x4F80_0000);
    assert_eq!(unsigned(0xFFFF_FF7F, 0), 0x4F7F_FFFF);
    assert_eq!(signed(1, 31), 0x3000_0000);
    assert_eq!(signed(0, 31), 0);
}

/// [AltiVec-PEM p:6-65 s:6.2] vctsxs saturates outside [-2^31, 2^31 - 1], and a word whose exponent + UIMM passes 254 saturates by its sign.
#[test]
fn signed_conversion_saturates_just_outside_the_word() {
    let s = |b, scale| vmx_to_signed(VmxMode::Java, b, scale);
    // 2^31 - 128, the largest binary32 below 2^31.
    assert_eq!(s(0x4EFF_FFFF, 0), (0x7FFF_FF80, false));
    assert_eq!(s(0x4F00_0000, 0), (0x7FFF_FFFF, true));
    assert_eq!(s(0xCF00_0000, 0), (0x8000_0000, false));
    assert_eq!(s(0xCF00_0001, 0), (0x8000_0000, true));
    // The largest finite value at the largest scale, and an infinity.
    assert_eq!(s(0x7F7F_FFFF, 31), (0x7FFF_FFFF, true));
    assert_eq!(s(0xFF80_0000, 0), (0x8000_0000, true));
    // -1.5 and 1.75 truncate toward zero.
    assert_eq!(s(0xBFC0_0000, 0), (0xFFFF_FFFF, false));
    assert_eq!(s(0x3FE0_0000, 0), (1, false));
}

#[test]
fn unsigned_conversion_saturates_below_zero_and_above_the_word() {
    let u = |b, scale| vmx_to_unsigned(VmxMode::Java, b, scale);
    assert_eq!(u(0x4F7F_FFFF, 0), (0xFFFF_FF00, false));
    assert_eq!(u(0x4F80_0000, 0), (0xFFFF_FFFF, true));
    assert_eq!(u(0xBF80_0000, 0), (0, true));
    assert_eq!(u(0x7F80_0000, 0), (0xFFFF_FFFF, true));
    // -0.5 truncates to zero before the range check.
    assert_eq!(u(0xBF00_0000, 0), (0, false));
}

/// [AltiVec-PEM p:3-15 s:3.2.5.4] every NaN converts to zero.
#[test]
fn a_nan_converts_to_zero_without_saturating() {
    for b in [0x7FC0_0000, 0xFF80_0001, 0x7F81_2345] {
        assert_eq!(vmx_to_signed(VmxMode::Java, b, 7), (0, false));
        assert_eq!(vmx_to_unsigned(VmxMode::Java, b, 7), (0, false));
    }
}

/// A denormal times 2^31 stays below one, so both modes give zero.
#[test]
fn a_denormal_truncates_to_zero_in_both_modes() {
    for mode in MODES {
        for b in [0x0000_0001, 0x007F_FFFF, 0x8000_0001, 0x807F_FFFF] {
            assert_eq!(vmx_to_signed(mode, b, 31), (0, false), "{mode:?} {b:#010x}");
            assert_eq!(
                vmx_to_unsigned(mode, b, 31),
                (0, false),
                "{mode:?} {b:#010x}"
            );
        }
    }
}

#[test]
#[ignore = "exhaustive over 2^32 words; run with --release --ignored"]
fn every_word_converts_like_the_host_at_scales_0_and_31() {
    for scale in [0, 31] {
        for b in 0..=u32::MAX {
            assert_eq!(
                vmx_to_signed(VmxMode::Java, b, scale),
                to_signed_reference(VmxMode::Java, b, scale),
                "vctsxs {b:#010x} x 2^{scale}"
            );
            assert_eq!(
                vmx_to_unsigned(VmxMode::Java, b, scale),
                to_unsigned_reference(VmxMode::Java, b, scale),
                "vctuxs {b:#010x} x 2^{scale}"
            );
            assert_eq!(
                vmx_from_integer(i64::from(b as i32), scale),
                from_reference(i64::from(b as i32), scale),
                "vcfsx {b:#010x} / 2^{scale}"
            );
            assert_eq!(
                vmx_from_integer(i64::from(b), scale),
                from_reference(i64::from(b), scale),
                "vcfux {b:#010x} / 2^{scale}"
            );
        }
    }
}
