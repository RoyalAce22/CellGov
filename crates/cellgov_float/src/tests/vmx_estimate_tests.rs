//! The four vector estimates against the documents' special values and
//! error bounds. The bounds guard a transcription error the console
//! words alone might miss.

use crate::vmx::VmxMode;
use crate::vmx_estimate::{
    vmx_exp2_estimate, vmx_log2_estimate, vmx_reciprocal_estimate, vmx_rsqrt_estimate,
};

const MODES: [VmxMode; 2] = [VmxMode::Java, VmxMode::NonJava];
const QNAN: u32 = 0x7FC0_0000;

/// Every word with biased exponent 1 to 254 at a stride, both signs.
fn normals(stride: u32) -> impl Iterator<Item = u32> {
    (0x0080_0000..0x7F80_0000u32)
        .step_by(stride as usize)
        .flat_map(|w| [w, w | 0x8000_0000])
}

/// [AltiVec-PEM p:6-123 s:6.2] the special-value table of vrefp.
#[test]
fn reciprocal_special_values() {
    for mode in MODES {
        let r = |b| vmx_reciprocal_estimate(mode, b);
        assert_eq!(r(0xFF80_0000), 0x8000_0000);
        assert_eq!(r(0x8000_0000), 0xFF80_0000);
        assert_eq!(r(0x0000_0000), 0x7F80_0000);
        assert_eq!(r(0x7F80_0000), 0x0000_0000);
        assert_eq!(r(0x7F81_2345), 0x7FC1_2345);
    }
    // NJ = 1 reads a denormal as a zero of its sign. NJ = 0 gives a
    // reciprocal of about -2^126, as the ppu_vmx_estimate_edges capture
    // recorded.
    assert_eq!(
        vmx_reciprocal_estimate(VmxMode::NonJava, 0x807F_FFFF),
        0xFF80_0000
    );
    assert_eq!(
        vmx_reciprocal_estimate(VmxMode::Java, 0x807F_FFFF),
        0xFE80_0106
    );
}

/// [AltiVec-PEM p:6-132 s:6.2] the special-value table of vrsqrtefp.
#[test]
fn rsqrt_special_values() {
    for mode in MODES {
        let r = |b| vmx_rsqrt_estimate(mode, b);
        assert_eq!(r(0xFF80_0000), QNAN);
        assert_eq!(r(0xBF80_0000), QNAN);
        assert_eq!(r(0x8000_0000), 0xFF80_0000);
        assert_eq!(r(0x0000_0000), 0x7F80_0000);
        assert_eq!(r(0x7F80_0000), 0x0000_0000);
        assert_eq!(r(0xFFC0_0042), 0xFFC0_0042);
    }
}

/// [AltiVec-PEM p:6-67 s:6.2] the special-value table of vexptefp.
#[test]
fn exp2_special_values() {
    for mode in MODES {
        let r = |b| vmx_exp2_estimate(mode, b);
        assert_eq!(r(0xFF80_0000), 0);
        assert_eq!(r(0x8000_0000), 0x3F80_0000);
        assert_eq!(r(0x0000_0000), 0x3F80_0000);
        assert_eq!(r(0x7F80_0000), 0x7F80_0000);
        assert_eq!(r(0x7F80_0001), 0x7FC0_0001);
    }
}

/// [AltiVec-PEM p:6-69 s:6.2] the special-value table of vlogefp.
#[test]
fn log2_special_values() {
    for mode in MODES {
        let r = |b| vmx_log2_estimate(mode, b);
        assert_eq!(r(0xFF80_0000), QNAN);
        assert_eq!(r(0xBF80_0000), QNAN);
        assert_eq!(r(0x8000_0000), 0xFF80_0000);
        assert_eq!(r(0x0000_0000), 0xFF80_0000);
        assert_eq!(r(0x7F80_0000), 0x7F80_0000);
    }
}

/// [AltiVec-PEM p:6-67 s:6.2] an integral x whose result is not 0 or +Inf gives the exact result.
#[test]
fn exp2_of_an_integer_is_exact() {
    for mode in MODES {
        for n in -126i32..=127 {
            let x = (n as f32).to_bits();
            assert_eq!(vmx_exp2_estimate(mode, x), ((n + 127) as u32) << 23, "{n}");
        }
        assert_eq!(vmx_exp2_estimate(mode, 128.0f32.to_bits()), 0x7F80_0000);
    }
}

/// The three floats just below -126 truncate to -126 but give +0, as
/// the ppu_vmx_estimate_sweep capture found; -126 itself gives 2^-126.
#[test]
fn exp2_of_a_value_below_minus_126_is_zero() {
    for mode in MODES {
        assert_eq!(vmx_exp2_estimate(mode, 0xC2FC_0000), 0x0080_0000);
        for b in 0xC2FC_0001..=0xC2FC_0003 {
            assert_eq!(vmx_exp2_estimate(mode, b), 0, "{b:#010x}");
        }
    }
}

/// [AltiVec-PEM p:6-123 s:6.2] the vrefp estimate is within 1/4096 of 1/x.
/// [AltiVec-PEM p:6-132 s:6.2] the vrsqrtefp estimate is within 1/4096 of 1/sqrt(x).
#[test]
fn reciprocal_estimates_meet_their_bound() {
    let mut checked = 0;
    for b in normals(97).filter(|b| b >> 23 & 0xFF < 253) {
        let x = f64::from(f32::from_bits(b));
        let r = f64::from(f32::from_bits(vmx_reciprocal_estimate(VmxMode::Java, b)));
        assert!((r * x - 1.0).abs() <= 1.0 / 4096.0, "vrefp {b:#010x}");
        if b >> 31 == 0 {
            let s = f64::from(f32::from_bits(vmx_rsqrt_estimate(VmxMode::Java, b)));
            assert!(
                (s * x.sqrt() - 1.0).abs() <= 1.0 / 4096.0,
                "vrsqrtefp {b:#010x}"
            );
        }
        checked += 1;
    }
    assert!(checked > 40_000_000 / 97 * 2 - 1000, "{checked}");
}

/// [AltiVec-PEM p:6-67 s:6.2] vexptefp is within 1/16 of 2^x.
/// [AltiVec-PEM p:6-69 s:6.2] vlogefp is within 2^-5 of log2(x).
#[test]
fn exp2_and_log2_estimates_meet_their_bounds() {
    for b in normals(97) {
        let x = f64::from(f32::from_bits(b));
        if (-126.0..128.0).contains(&x) {
            let e = f64::from(f32::from_bits(vmx_exp2_estimate(VmxMode::Java, b)));
            assert!(
                (e / x.exp2() - 1.0).abs() <= 1.0 / 16.0,
                "vexptefp {b:#010x}"
            );
        }
        if x > 0.0 {
            let l = f64::from(f32::from_bits(vmx_log2_estimate(VmxMode::Java, b)));
            assert!((l - x.log2()).abs() <= 1.0 / 32.0, "vlogefp {b:#010x}");
        }
    }
}

/// [AltiVec-PEM p:6-67 s:6.2], [AltiVec-PEM p:6-69 s:6.2] the 12 most significant bits of the result significand are monotonic.
#[test]
fn exp2_and_log2_estimates_are_monotonic() {
    let ordered = |w: u32| {
        if w >> 31 == 1 {
            -i64::from(w & 0x7FFF_FFFF)
        } else {
            i64::from(w)
        }
    };
    let mut last_exp = i64::MIN;
    let mut last_log = i64::MIN;
    for b in (0x0080_0000..0x4300_0000u32).step_by(13) {
        // Sign, exponent and the top 11 fraction bits: the implicit bit
        // and 11 fraction bits are the 12 significand bits.
        let e = ordered(vmx_exp2_estimate(VmxMode::Java, b) & 0xFFFF_F000);
        let l = ordered(vmx_log2_estimate(VmxMode::Java, b) & 0xFFFF_F000);
        assert!(e >= last_exp, "vexptefp {b:#010x}");
        assert!(l >= last_log, "vlogefp {b:#010x}");
        last_exp = e;
        last_log = l;
    }
}
