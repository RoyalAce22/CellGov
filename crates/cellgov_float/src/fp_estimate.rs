//! The two scalar estimates, `fres` and `frsqrte`, as the PPE computes
//! them.
//!
//! The documents give only error bounds, and allow each implementation
//! its own values. The `ppu_fp_estimate_readout` console capture read the
//! tables, the input bits each estimate uses and its exponent range off
//! a CECH-2001A. The capture read operands with a biased exponent of 1023
//! under all four FPSCR rounding modes, and neither estimate depends on
//! the mode for them. It read the overflow and underflow edges under
//! RN = 0 alone.

/// `fres` result fractions, in units of 2^-23 of a single fraction, for
/// each value of the top seven fraction bits of the operand.
const RECIPROCAL: [u64; 128] = [
    0x7F_0000, 0x7D_0800, 0x7B_1800, 0x79_3000, 0x77_5000, 0x75_7000, 0x73_A000, 0x71_E000,
    0x70_0000, 0x6E_4000, 0x6C_A000, 0x6A_E000, 0x69_4000, 0x67_8000, 0x66_0000, 0x64_6000,
    0x62_C000, 0x61_4000, 0x5F_C000, 0x5E_4000, 0x5C_C000, 0x5B_4000, 0x59_C000, 0x58_4000,
    0x57_0000, 0x55_8000, 0x54_0000, 0x52_C000, 0x51_8000, 0x50_0000, 0x4E_C000, 0x4D_8000,
    0x4C_0000, 0x4B_0000, 0x49_8000, 0x48_8000, 0x47_4000, 0x46_0000, 0x44_C000, 0x43_8000,
    0x42_8000, 0x41_8000, 0x40_0000, 0x3F_0000, 0x3E_0000, 0x3D_0000, 0x3B_C000, 0x3A_C000,
    0x39_8000, 0x38_8000, 0x37_8000, 0x36_8000, 0x35_8000, 0x34_8000, 0x33_8000, 0x32_8000,
    0x31_8000, 0x30_8000, 0x2F_8000, 0x2E_C000, 0x2E_0000, 0x2D_0000, 0x2C_0000, 0x2B_0000,
    0x2A_0000, 0x29_8000, 0x28_8000, 0x27_8000, 0x26_C000, 0x26_0000, 0x25_0000, 0x24_4000,
    0x23_8000, 0x22_8000, 0x22_0000, 0x21_0000, 0x20_0000, 0x1F_8000, 0x1E_8000, 0x1E_0000,
    0x1D_0000, 0x1C_8000, 0x1B_8000, 0x1B_0000, 0x1A_0000, 0x19_8000, 0x19_0000, 0x18_0000,
    0x17_8000, 0x16_8000, 0x16_0000, 0x15_8000, 0x14_8000, 0x14_0000, 0x13_8000, 0x12_8000,
    0x12_0000, 0x11_8000, 0x10_8000, 0x10_0000, 0x0F_8000, 0x0F_0000, 0x0E_0000, 0x0D_8000,
    0x0D_0000, 0x0C_8000, 0x0B_8000, 0x0B_0000, 0x0A_8000, 0x0A_0000, 0x09_8000, 0x09_0000,
    0x08_0000, 0x07_8000, 0x07_0000, 0x06_8000, 0x06_0000, 0x05_8000, 0x05_0000, 0x04_8000,
    0x04_0000, 0x03_8000, 0x03_0000, 0x02_8000, 0x02_0000, 0x01_8000, 0x01_0000, 0x00_0000,
];

/// `frsqrte` result fractions for an even unbiased exponent, in units of
/// 2^-23 of a single fraction, for each value of the top three fraction
/// bits of the operand.
const RSQRT_EVEN_POWER: [u64; 8] = [
    0x78_8000, 0x6C_0000, 0x60_0000, 0x54_0000, 0x4C_0000, 0x44_0000, 0x40_0000, 0x38_0000,
];

/// `frsqrte` result fractions for an odd unbiased exponent.
const RSQRT_ODD_POWER: [u64; 8] = [
    0x30_0000, 0x26_0000, 0x1E_0000, 0x18_0000, 0x10_0000, 0x0C_0000, 0x08_0000, 0x04_0000,
];

const SIGN: u64 = 1 << 63;
const INFINITY: u64 = 0x7FF0_0000_0000_0000;
const FRACTION: u64 = (1 << 52) - 1;
const QUIET: u64 = 1 << 51;
const DEFAULT_NAN: u64 = 0x7FF8_0000_0000_0000;

/// `fres` of the double `b`.
///
/// [PPC-Book1 p:125 s:5.2.1.1] -Inf gives -0, -0 gives -Inf, +0 gives +Inf, +Inf gives +0 and a QNaN is returned unchanged.
/// [PPC-Book1 p:91 s:4.3.2] an SNaN operand gives its quieted copy.
/// The `ppu_fp_estimate_readout` capture shows that a result exponent above 127 gives an infinity and one below -150 gives a zero.
/// Between those edges, the result does not denormalize to single precision.
pub fn fres_estimate(b: u64) -> u64 {
    if is_nan(b) {
        return b | QUIET;
    }
    let sign = b & SIGN;
    let biased = (b >> 52 & 0x7FF) as i32;
    if b & !SIGN == INFINITY {
        return sign;
    }
    if biased == 0 {
        // A zero, or a denormal whose reciprocal is past the single range.
        return sign | INFINITY;
    }
    let power = 1022 - biased;
    if power > 127 {
        return sign | INFINITY;
    }
    if power < -150 {
        return sign;
    }
    sign | ((power + 1023) as u64) << 52 | RECIPROCAL[(b >> 45 & 0x7F) as usize] << 29
}

/// `frsqrte` of the double `b`.
///
/// [PPC-Book1 p:126 s:5.2.1.1] -Inf and every value below 0 give a QNaN, -0 gives -Inf, +0 gives +Inf, +Inf gives +0 and a QNaN is returned unchanged.
/// [PPC-Book1 p:91 s:4.3.2] an SNaN operand gives its quieted copy, and the generated QNaN is 0x7FF8_0000_0000_0000.
/// The `ppu_fp_estimate_readout` capture shows the result in double range: the estimate normalizes a denormal operand first.
pub fn frsqrte_estimate(b: u64) -> u64 {
    if is_nan(b) {
        return b | QUIET;
    }
    if b & !SIGN == 0 {
        return b | INFINITY;
    }
    if b & SIGN != 0 {
        return DEFAULT_NAN;
    }
    if b == INFINITY {
        return 0;
    }
    let (exponent, fraction) = normalized(b);
    let index = (fraction >> 49) as usize;
    let (table, power) = if exponent % 2 == 0 {
        (&RSQRT_EVEN_POWER, -(exponent / 2) - 1)
    } else {
        (&RSQRT_ODD_POWER, -((exponent - 1) >> 1) - 1)
    };
    ((power + 1023) as u64) << 52 | table[index] << 29
}

fn is_nan(b: u64) -> bool {
    b & INFINITY == INFINITY && b & FRACTION != 0
}

/// The unbiased exponent and 52-bit fraction of a nonzero finite double,
/// with a denormal normalized first.
fn normalized(b: u64) -> (i32, u64) {
    let biased = (b >> 52 & 0x7FF) as i32;
    let fraction = b & FRACTION;
    if biased != 0 {
        return (biased - 1023, fraction);
    }
    let top = 63 - fraction.leading_zeros();
    (top as i32 - 1074, (fraction << (52 - top)) & FRACTION)
}
