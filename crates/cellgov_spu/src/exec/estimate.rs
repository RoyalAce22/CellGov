//! The reciprocal and reciprocal-square-root estimates: a sign, an
//! exponent map and one table entry per segment of the significand.
//!
//! The architecture lets each implementation give different results
//! [SPU-ISA p:195 s:9], and no public document gives the CBE's table
//! contents. These tables are CellGov's own: each entry is the minimax line of its segment,
//! written as a base less a step times the interpolation fraction and
//! rounded to the entry's field widths. They meet the documented
//! Newton-Raphson bounds and the documented `frsqest` zero threshold, but
//! they are not the hardware's table, and the index widths are the
//! smallest that meet those bounds. Hardware vectors replace them.

/// Fraction bits `frest` indexes its table by.
const FREST_INDEX_BITS: u32 = 5;

/// Fraction bits `frsqest` indexes each of its two tables by.
const FRSQEST_INDEX_BITS: u32 = 4;

/// `frest` entries, (BaseFraction, StepFraction), for 2/m over each
/// segment of the significand m.
const FREST_TABLE: [(u16, u16); 1 << FREST_INDEX_BITS] = [
    (8190, 993),
    (8161, 935),
    (7227, 881),
    (7202, 832),
    (6370, 787),
    (6350, 746),
    (5604, 708),
    (5586, 672),
    (4914, 639),
    (4899, 609),
    (4290, 581),
    (4277, 554),
    (3723, 530),
    (3711, 507),
    (3205, 485),
    (3195, 465),
    (2730, 446),
    (2721, 428),
    (2293, 411),
    (2285, 395),
    (1890, 380),
    (1883, 366),
    (1517, 353),
    (1510, 340),
    (1170, 329),
    (1164, 317),
    (847, 306),
    (842, 296),
    (546, 286),
    (541, 277),
    (264, 268),
    (260, 260),
];

/// `frsqest` entries for an odd biased exponent (an even power of two):
/// 2/sqrt(m) over each segment of m.
const FRSQEST_ODD_EXPONENT: [(u16, u16); 1 << FRSQEST_INDEX_BITS] = [
    (8189, 489),
    (7700, 448),
    (7253, 412),
    (6841, 381),
    (6461, 353),
    (6108, 329),
    (5779, 307),
    (5472, 288),
    (5184, 270),
    (4914, 255),
    (4660, 240),
    (4420, 227),
    (4192, 215),
    (3977, 205),
    (3773, 195),
    (3578, 185),
];

/// `frsqest` entries for an even biased exponent (an odd power of two):
/// 2/sqrt(2m) over each segment of m.
// [SPU-ISA p:217 s:9] with an exponent-0 input, y2 is 0x7fffffff exactly up to fraction 0x000ff53c; entry 1 is the one line near its segment's minimax line that does so.
const FRSQEST_EVEN_EXPONENT: [(u16, u16); 1 << FRSQEST_INDEX_BITS] = [
    (3391, 346),
    (3046, 317),
    (2729, 291),
    (2438, 269),
    (2169, 250),
    (1919, 233),
    (1687, 217),
    (1470, 203),
    (1267, 191),
    (1076, 180),
    (896, 170),
    (726, 161),
    (565, 152),
    (413, 145),
    (268, 138),
    (131, 131),
];

/// The table index of `x`: its top `bits` fraction bits.
fn index(x: u32, bits: u32) -> usize {
    (x >> (23 - bits) & ((1 << bits) - 1)) as usize
}

// [SPU-ISA p:215 s:9] S in bit 0, the biased exponent in bits 1:8, BaseFraction in 9:21 and StepFraction in 22:31.
fn pack(negative: bool, exponent: u32, (base, step): (u16, u16)) -> u32 {
    u32::from(negative) << 31 | exponent << 23 | u32::from(base) << 10 | u32::from(step)
}

/// The biased exponent field of `x`.
fn exponent(x: u32) -> u32 {
    x >> 23 & 0xFF
}

/// The `frest` result for one slot.
// [SPU-ISA p:215 s:9] 1/0 gives 0x7FFFFFFF after the sequence, so a zero exponent maps to 255; [SPU-ISA p:216 s:9] every |x| >= 2^126 underflows, so exponents from 253 up map to 0.
pub(super) fn frest(x: u32) -> u32 {
    let e = exponent(x);
    // 1/(m x 2^(e-127)) = (2/m) x 2^((253-e)-127).
    let result_exponent = if e == 0 {
        255
    } else {
        253u32.saturating_sub(e)
    };
    pack(
        x >> 31 == 1,
        result_exponent,
        FREST_TABLE[index(x, FREST_INDEX_BITS)],
    )
}

/// The `frsqest` result for one slot; the sign is always 0.
// [SPU-ISA p:217 s:9] the estimate is of 1/sqrt(abs(x)), and a zero exponent gives y2 >= 0x7fc00000 after the sequence, so it maps to 255.
pub(super) fn frsqest(x: u32) -> u32 {
    let e = exponent(x);
    // 1/sqrt(m x 2^E) = (2/sqrt(m)) x 2^(-E/2 - 1) for even E, and
    // (2/sqrt(2m)) x 2^(-(E-1)/2 - 1) for odd E.
    let result_exponent = if e == 0 {
        255
    } else {
        (126 - (e as i32 - 127).div_euclid(2)) as u32
    };
    let table = if e & 1 == 1 {
        &FRSQEST_ODD_EXPONENT
    } else {
        &FRSQEST_EVEN_EXPONENT
    };
    pack(false, result_exponent, table[index(x, FRSQEST_INDEX_BITS)])
}
