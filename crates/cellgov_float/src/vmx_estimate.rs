//! The four vector estimates, `vrefp`, `vrsqrtefp`, `vexptefp` and
//! `vlogefp`, as the PPE computes them.
//!
//! The documents give only error bounds, and allow each implementation
//! its own values. The `ppu_vmx_estimate_readout` and
//! `ppu_vmx_estimate_edges` console captures read the tables, the input
//! bits each estimate uses, its exponent map and its rounding off a
//! CECH-2001A. The `ppu_vmx_estimate_sweep` capture compared this model
//! with the instructions over every input.

use crate::vmx::{vmx_nan, VmxMode, VMX_DEFAULT_NAN};

/// `vrefp` segments: the result fraction at the segment start, and the
/// drop per 2^-14 of input fraction, times 4. The top five fraction bits
/// pick the segment.
const RECIPROCAL: [(u32, u32); 32] = [
    (0x7F_F800, 3972),
    (0x78_3800, 3740),
    (0x70_EA00, 3524),
    (0x6A_0800, 3328),
    (0x63_8800, 3148),
    (0x5D_6200, 2984),
    (0x57_9000, 2832),
    (0x52_0800, 2688),
    (0x4C_C800, 2556),
    (0x47_CA00, 2436),
    (0x43_0800, 2324),
    (0x3E_8000, 2216),
    (0x3A_2C00, 2120),
    (0x36_0800, 2028),
    (0x32_1400, 1940),
    (0x2E_4A00, 1860),
    (0x2A_A800, 1784),
    (0x27_2C00, 1712),
    (0x23_D600, 1644),
    (0x20_9E00, 1580),
    (0x1D_8800, 1520),
    (0x1A_9000, 1464),
    (0x17_AE00, 1388),
    (0x14_F800, 1388),
    (0x12_4400, 1292),
    (0x0F_BE00, 1292),
    (0x0D_3800, 1204),
    (0x0A_DE00, 1204),
    (0x08_8400, 1128),
    (0x06_5000, 1128),
    (0x04_1C00, 1056),
    (0x02_0C00, 1048),
];

/// `vrsqrtefp` segments for an even unbiased exponent, laid out as
/// [`RECIPROCAL`]; the top four fraction bits pick the segment.
const RSQRT_EVEN_POWER: [(u32, u32); 16] = [
    (0x7F_F400, 1956),
    (0x78_5200, 1792),
    (0x71_5400, 1648),
    (0x6A_E400, 1522),
    (0x64_F200, 1412),
    (0x5F_6E00, 1316),
    (0x5A_4C00, 1228),
    (0x55_8000, 1150),
    (0x51_0200, 1082),
    (0x4C_CA00, 1018),
    (0x48_D000, 962),
    (0x45_0E00, 910),
    (0x41_8200, 862),
    (0x3E_2400, 818),
    (0x3A_F200, 778),
    (0x37_E800, 742),
];

/// `vrsqrtefp` segments for an odd unbiased exponent.
const RSQRT_ODD_POWER: [(u32, u32); 16] = [
    (0x34_FD00, 1384),
    (0x2F_9700, 1267),
    (0x2A_A500, 1165),
    (0x26_1800, 1077),
    (0x21_E400, 999),
    (0x1D_FE00, 930),
    (0x1A_5C00, 869),
    (0x16_F800, 814),
    (0x13_CA00, 764),
    (0x10_CE00, 720),
    (0x0D_FE00, 680),
    (0x0B_5700, 643),
    (0x08_D400, 609),
    (0x06_7300, 579),
    (0x04_3100, 550),
    (0x02_0B00, 523),
];

/// `vlogefp` segments of 128 table entries: the value at the segment
/// start in units of 2^-11, then a shift, a slope and an offset, so that
/// entry `i` is `base + (slope * (i % 128) + offset) >> shift`.
const LOG2: [(u32, u32, u32, u32); 16] = [
    (0, 1, 3, 0),
    (192, 2, 5, 0),
    (352, 2, 5, 0),
    (512, 3, 9, 0),
    (656, 3, 9, 0),
    (800, 3, 9, 0),
    (944, 0, 1, 0),
    (1072, 0, 1, 0),
    (1200, 0, 1, 0),
    (1327, 3, 7, 7),
    (1439, 3, 7, 7),
    (1551, 3, 7, 7),
    (1663, 2, 3, 3),
    (1759, 2, 3, 3),
    (1855, 2, 3, 3),
    (1951, 2, 3, 3),
];

const SIGN: u32 = 0x8000_0000;
const INFINITY: u32 = 0x7F80_0000;
const ONE: u32 = 0x3F80_0000;
const MIN_NORMAL: u32 = 0x0080_0000;
/// The magnitude bits of 126.0.
const MINUS_126_MAGNITUDE: u32 = 0x42FC_0000;

/// `vrefp` of the binary32 word `b` in `mode`.
///
/// [AltiVec-PEM p:6-123 s:6.2] -Inf gives -0, -0 gives -Inf, +0 gives +Inf and +Inf gives +0; NJ = 1 reads a denormal operand as a zero of its sign.
/// The `ppu_vmx_estimate_edges` capture shows that a result below 2^-126 rounds to nearest even as it denormalizes.
/// It also shows that 2^126 gives 2^-126.
pub fn vmx_reciprocal_estimate(mode: VmxMode, b: u32) -> u32 {
    if let Some(nan) = vmx_nan(&[b]) {
        return nan;
    }
    let sign = b & SIGN;
    if reads_as_zero(mode, b) {
        return sign | INFINITY;
    }
    if b & !SIGN == INFINITY {
        return sign;
    }
    let (exponent, fraction) = normalized(b);
    if exponent == 126 && fraction == 0 {
        // 1/x is 2^-126 exactly, which is not tiny.
        return sign | MIN_NORMAL;
    }
    let (base, slope) = RECIPROCAL[(fraction >> 18) as usize];
    let step = (fraction & 0x3_FFFF) >> 9;
    pack(mode, sign, -exponent - 1, base - slope * step / 4)
}

/// `vrsqrtefp` of the binary32 word `b` in `mode`.
///
/// [AltiVec-PEM p:6-132 s:6.2] -Inf and every value below 0 give a QNaN, -0 gives -Inf, +0 gives +Inf and +Inf gives +0.
/// [AltiVec-PEM p:3-13 s:3.2.4.2] the QNaN of an invalid operation is 0x7FC0_0000.
/// [AltiVec-PEM p:3-11 s:3.2.1] NJ = 1 reads a denormal operand as a zero of its sign.
pub fn vmx_rsqrt_estimate(mode: VmxMode, b: u32) -> u32 {
    if let Some(nan) = vmx_nan(&[b]) {
        return nan;
    }
    if reads_as_zero(mode, b) {
        return b & SIGN | INFINITY;
    }
    if b & SIGN != 0 {
        return VMX_DEFAULT_NAN;
    }
    if b == INFINITY {
        return 0;
    }
    let (exponent, fraction) = normalized(b);
    let (table, power) = if exponent % 2 == 0 {
        (&RSQRT_EVEN_POWER, -(exponent / 2) - 1)
    } else {
        (&RSQRT_ODD_POWER, -((exponent - 1) >> 1) - 1)
    };
    let (base, slope) = table[(fraction >> 19) as usize];
    let step = (fraction & 0x7_FFFF) >> 9;
    // The interpolation is in quarter units, rounded to nearest even.
    let quarters = 4 * base - slope * step;
    let (whole, rest) = (quarters >> 2, quarters & 3);
    let rounded = whole + u32::from(rest > 2 || (rest == 2 && whole & 1 == 1));
    pack(mode, 0, power, rounded)
}

/// `vexptefp` of the binary32 word `b` in `mode`.
///
/// [AltiVec-PEM p:6-67 s:6.2] -Inf gives +0, -0 and +0 give +1, and +Inf gives +Inf.
/// The `ppu_vmx_estimate_readout` capture shows that the estimate reads the input as a fixed-point number with 15 fraction bits, truncated toward zero.
/// The `ppu_vmx_estimate_edges` capture shows that every result below 2^-126 is +0 under both NJ values.
/// The `ppu_vmx_estimate_sweep` capture shows that every x below -126 gives +0.
pub fn vmx_exp2_estimate(mode: VmxMode, b: u32) -> u32 {
    if let Some(nan) = vmx_nan(&[b]) {
        return nan;
    }
    if b == SIGN | INFINITY {
        return 0;
    }
    if b == INFINITY {
        return INFINITY;
    }
    if reads_as_zero(mode, b) {
        return ONE;
    }
    if b & SIGN != 0 && b & !SIGN > MINUS_126_MAGNITUDE {
        // Every x below -126 gives +0, even one that truncates to -126.
        return 0;
    }
    let (exponent, fraction) = normalized(b);
    let shift = exponent + 15 - 23;
    let magnitude = i64::from(fraction | 0x80_0000);
    let fixed = match shift {
        8.. => 1 << 30,
        0..=7 => magnitude << shift,
        -63..=-1 => magnitude >> -shift,
        _ => 0,
    };
    if fixed >= 1 << 30 {
        return if b & SIGN != 0 { 0 } else { INFINITY };
    }
    let fixed = if b & SIGN != 0 { -fixed } else { fixed };
    let power = (fixed >> 15) as i32;
    if power < -126 {
        return 0;
    }
    pack(mode, 0, power, exp2_fraction((fixed & 0x7FFF) as u32) << 12)
}

/// `vlogefp` of the binary32 word `b` in `mode`.
///
/// [AltiVec-PEM p:6-69 s:6.2] -Inf and every value below 0 give a QNaN, +0 and -0 give -Inf, and +Inf gives +Inf.
/// [AltiVec-PEM p:3-13 s:3.2.4.2] the QNaN of an invalid operation is 0x7FC0_0000.
/// The `ppu_vmx_estimate_readout` capture shows that the result is the unbiased exponent plus a table entry in units of 2^-11.
/// The top 11 fraction bits pick the entry.
pub fn vmx_log2_estimate(mode: VmxMode, b: u32) -> u32 {
    if let Some(nan) = vmx_nan(&[b]) {
        return nan;
    }
    if reads_as_zero(mode, b) {
        return SIGN | INFINITY;
    }
    if b & SIGN != 0 {
        return VMX_DEFAULT_NAN;
    }
    if b == INFINITY {
        return INFINITY;
    }
    let (exponent, fraction) = normalized(b);
    let index = fraction >> 12;
    let (base, shift, slope, offset) = LOG2[(index >> 7) as usize];
    let entry = base + ((slope * (index & 127) + offset) >> shift);
    let value = exponent * 2048 + entry as i32;
    if value == 0 {
        return 0;
    }
    // Every value fits in 24 bits, so the conversion is exact.
    let magnitude = value.unsigned_abs();
    let top = 31 - magnitude.leading_zeros();
    let fraction = (magnitude << (23 - top)) & 0x7F_FFFF;
    u32::from(value < 0) << 31 | (top + 116) << 23 | fraction
}

/// The table fraction of 2^(f / 2^15), in units of 2^-11.
fn exp2_fraction(f: u32) -> u32 {
    match f {
        0..8192 => (f >> 4) - (f >> 6),
        8192..14336 => (f >> 4) - (f >> 7) - 64,
        14336..20480 => (f >> 4) - 176,
        20480..26624 => (f >> 4) + (f >> 7) - 335,
        26624..30720 => (f >> 4) + (f >> 6) - 543,
        _ => (f >> 4) + (f >> 5) - 1023,
    }
}

/// Whether `mode` reads `b` as a zero: a zero, or a denormal under NJ = 1.
fn reads_as_zero(mode: VmxMode, b: u32) -> bool {
    b & !SIGN == 0 || (mode == VmxMode::NonJava && b & INFINITY == 0)
}

/// The unbiased exponent and 23-bit fraction of a nonzero finite word,
/// with a denormal normalized first.
fn normalized(b: u32) -> (i32, u32) {
    let biased = (b >> 23 & 0xFF) as i32;
    let fraction = b & 0x7F_FFFF;
    if biased != 0 {
        return (biased - 127, fraction);
    }
    let top = 31 - fraction.leading_zeros();
    (top as i32 - 149, (fraction << (23 - top)) & 0x7F_FFFF)
}

/// Packs `sign`, 1.`fraction` x 2^`power` into a binary32 word.
///
/// A power below -126 denormalizes with round to nearest even. Under
/// NJ = 1, it gives a zero of its sign.
fn pack(mode: VmxMode, sign: u32, power: i32, fraction: u32) -> u32 {
    let biased = power + 127;
    if biased >= 255 {
        return sign | INFINITY;
    }
    if biased >= 1 {
        return sign | (biased as u32) << 23 | fraction;
    }
    let shift = (1 - biased) as u32;
    if mode == VmxMode::NonJava || shift > 24 {
        return sign;
    }
    let significand = fraction | 0x80_0000;
    let kept = significand >> shift;
    let rest = significand & ((1 << shift) - 1);
    let half = 1 << (shift - 1);
    sign | (kept + u32::from(rest > half || (rest == half && kept & 1 == 1)))
}
