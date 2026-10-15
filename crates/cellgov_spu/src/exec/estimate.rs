//! The reciprocal and reciprocal-square-root estimates: a sign, an
//! exponent map and one table entry per segment of the significand.
//!
//! The architecture lets each implementation give different results
//! [SPU-ISA p:195 s:9], and no public document gives the CBE's table
//! contents. Each estimate writes its entry into its result, so the
//! `spu_estimate_readout` microtest read the tables, the index widths
//! and the exponent maps here off a CECH-2001A. The `spu_estimate_sweep`
//! microtest compared this model with both instructions on that console
//! over all 2^32 inputs and found no difference.

/// The fraction bits that index each table.
const INDEX_BITS: u32 = 5;

/// `frest` entries, (BaseFraction, StepFraction), for 2/m over each
/// segment of the significand m.
const FREST_TABLE: [(u16, u16); 1 << INDEX_BITS] = [
    (8190, 992),
    (8161, 934),
    (7227, 882),
    (7202, 832),
    (6370, 786),
    (6350, 746),
    (5604, 708),
    (5586, 672),
    (4914, 638),
    (4900, 610),
    (4290, 580),
    (4277, 554),
    (3723, 530),
    (3711, 506),
    (3205, 484),
    (3196, 466),
    (2730, 446),
    (2721, 428),
    (2293, 410),
    (2287, 398),
    (1889, 374),
    (1889, 374),
    (1515, 346),
    (1515, 346),
    (1169, 323),
    (1169, 323),
    (846, 301),
    (846, 301),
    (545, 282),
    (545, 282),
    (263, 263),
    (263, 263),
];

/// `frsqest` entries for an odd biased exponent (an even power of two):
/// 2/sqrt(m) over each segment of m.
const FRSQEST_ODD_EXPONENT: [(u16, u16); 1 << INDEX_BITS] = [
    (8191, 500),
    (8180, 478),
    (7702, 456),
    (7695, 442),
    (7253, 412),
    (7253, 412),
    (6841, 380),
    (6841, 380),
    (6461, 353),
    (6461, 353),
    (6108, 329),
    (6108, 329),
    (5779, 307),
    (5779, 307),
    (5472, 287),
    (5472, 287),
    (5185, 271),
    (5185, 271),
    (4914, 254),
    (4914, 254),
    (4660, 240),
    (4660, 240),
    (4420, 228),
    (4420, 228),
    (4192, 215),
    (4192, 215),
    (3977, 204),
    (3977, 204),
    (3773, 195),
    (3773, 195),
    (3578, 186),
    (3578, 186),
];

/// `frsqest` entries for an even biased exponent (an odd power of two):
/// 2/sqrt(2m) over each segment of m.
///
/// [SPU-ISA p:217 s:9] with an exponent-0 input, y2 is 0x7fffffff exactly up to fraction 0x000ff53c.
const FRSQEST_EVEN_EXPONENT: [(u16, u16); 1 << INDEX_BITS] = [
    (3392, 352),
    (3386, 340),
    (3046, 317),
    (3046, 317),
    (2729, 291),
    (2729, 291),
    (2438, 269),
    (2438, 269),
    (2169, 249),
    (2169, 249),
    (1920, 233),
    (1920, 233),
    (1687, 217),
    (1687, 217),
    (1470, 203),
    (1470, 203),
    (1267, 192),
    (1267, 192),
    (1075, 179),
    (1075, 179),
    (896, 170),
    (896, 170),
    (726, 161),
    (726, 161),
    (565, 152),
    (565, 152),
    (413, 145),
    (413, 145),
    (268, 137),
    (268, 137),
    (131, 131),
    (131, 131),
];

/// The table index of `x`: its top `bits` fraction bits.
fn index(x: u32, bits: u32) -> usize {
    (x >> (23 - bits) & ((1 << bits) - 1)) as usize
}

/// [SPU-ISA p:215 s:9] S in bit 0, the biased exponent in bits 1:8, BaseFraction in 9:21 and StepFraction in 22:31.
fn pack(negative: bool, exponent: u32, (base, step): (u16, u16)) -> u32 {
    u32::from(negative) << 31 | exponent << 23 | u32::from(base) << 10 | u32::from(step)
}

/// The biased exponent field of `x`.
fn exponent(x: u32) -> u32 {
    x >> 23 & 0xFF
}

/// The `frest` result for one slot.
///
/// [SPU-ISA p:215 s:9] 1/0 gives 0x7FFFFFFF after the sequence, so a zero exponent maps to 255; [SPU-ISA p:216 s:9] every |x| >= 2^126 underflows, so exponents from 253 up map to 0.
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
        FREST_TABLE[index(x, INDEX_BITS)],
    )
}

/// The `frsqest` result for one slot; the sign is always 0.
///
/// [SPU-ISA p:217 s:9] the estimate is of 1/sqrt(abs(x)), and a zero exponent gives y2 >= 0x7fc00000 after the sequence, so it maps to 255.
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
    pack(false, result_exponent, table[index(x, INDEX_BITS)])
}
