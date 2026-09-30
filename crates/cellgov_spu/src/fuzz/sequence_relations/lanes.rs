//! Lane arithmetic the fused references compute with: element access by
//! width, and the compares written from the ISA pages.
//!
//! [Liu2024 p:326:8 s:3] A vector operation's semantics is a function of
//! each lane, so a fused form states its result lane by lane.

use crate::state::SpuState;

/// The register the symbolic register `symbolic` names under `assignment`.
pub(super) fn reg(state: &SpuState, assignment: &[u8], symbolic: u8) -> [u8; 16] {
    state.regs[usize::from(assignment[usize::from(symbolic)])]
}

/// Writes `value` to the register `symbolic` names.
pub(super) fn set(state: &mut SpuState, assignment: &[u8], symbolic: u8, value: [u8; 16]) {
    state.set_reg(usize::from(assignment[usize::from(symbolic)]), value);
}

/// The four words of `value`, preferred word first.
pub(super) fn words(value: [u8; 16]) -> [u32; 4] {
    std::array::from_fn(|i| {
        u32::from_be_bytes([
            value[4 * i],
            value[4 * i + 1],
            value[4 * i + 2],
            value[4 * i + 3],
        ])
    })
}

/// The register whose words are `words`.
pub(super) fn from_words(words: [u32; 4]) -> [u8; 16] {
    std::array::from_fn(|byte| words[byte / 4].to_be_bytes()[byte % 4])
}

/// The eight halfwords of `value`.
pub(super) fn halves(value: [u8; 16]) -> [u16; 8] {
    std::array::from_fn(|i| u16::from_be_bytes([value[2 * i], value[2 * i + 1]]))
}

/// The register whose halfwords are `halves`.
pub(super) fn from_halves(halves: [u16; 8]) -> [u8; 16] {
    std::array::from_fn(|byte| halves[byte / 2].to_be_bytes()[byte % 2])
}

/// All ones when `holds`, zero otherwise, as a word.
pub(super) fn mask32(holds: bool) -> u32 {
    if holds {
        u32::MAX
    } else {
        0
    }
}

/// All ones when `holds`, zero otherwise, as a halfword.
pub(super) fn mask16(holds: bool) -> u16 {
    if holds {
        u16::MAX
    } else {
        0
    }
}

/// All ones when `holds`, zero otherwise, as a byte.
pub(super) fn mask8(holds: bool) -> u8 {
    if holds {
        u8::MAX
    } else {
        0
    }
}

/// The order key of a single-precision word: zero for any zero exponent,
/// the signed magnitude otherwise.
///
/// [SPU-ISA p:196 s:9] The SPU treats denorms as zero; the extended range has
/// no infinity or NaN, so every other exponent orders by magnitude.
/// [SPU-ISA p:231 s:9 Fceq] Two zeros compare equal whatever their
/// fractions and signs.
pub(super) fn float_key(bits: u32) -> i64 {
    if bits & 0x7F80_0000 == 0 {
        0
    } else {
        let magnitude = i64::from(bits & 0x7FFF_FFFF);
        if bits & 0x8000_0000 != 0 {
            -magnitude
        } else {
            magnitude
        }
    }
}

/// The compare a select or splat row applies, lane by lane.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Compare {
    /// Equal.
    Equal,
    /// Signed greater than.
    Greater,
    /// Unsigned greater than.
    LogicalGreater,
    /// [SPU-ISA p:231 s:9 Fceq] single-precision equal.
    FloatEqual,
    /// [SPU-ISA p:233 s:9 Fcgt] single-precision greater than.
    FloatGreater,
    /// [SPU-ISA p:232 s:9 Fcmeq] magnitudes equal.
    MagnitudeEqual,
    /// [SPU-ISA p:234 s:9 Fcmgt] magnitude greater than.
    MagnitudeGreater,
}

/// The width of the lanes a compare works on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Width {
    /// Sixteen bytes.
    Byte,
    /// Eight halfwords.
    Half,
    /// Four words.
    Word,
}

/// The lane mask of `compare` on `x` against `y`, at `width`.
///
/// [SPU-ISA p:156 s:7] through [SPU-ISA p:173 s:7]: each compare yields all
/// ones in a lane where it holds and zero elsewhere; the `cgt` forms
/// compare signed, the `clgt` forms unsigned.
pub(super) fn compare_mask(compare: Compare, width: Width, x: [u8; 16], y: [u8; 16]) -> [u8; 16] {
    match width {
        Width::Word => {
            let (x, y) = (words(x), words(y));
            from_words(std::array::from_fn(|i| {
                mask32(holds32(compare, x[i], y[i]))
            }))
        }
        Width::Half => {
            let (x, y) = (halves(x), halves(y));
            from_halves(std::array::from_fn(|i| {
                mask16(match compare {
                    Compare::Equal => x[i] == y[i],
                    Compare::Greater => (x[i] as i16) > (y[i] as i16),
                    _ => x[i] > y[i],
                })
            }))
        }
        Width::Byte => std::array::from_fn(|i| {
            mask8(match compare {
                Compare::Equal => x[i] == y[i],
                Compare::Greater => (x[i] as i8) > (y[i] as i8),
                _ => x[i] > y[i],
            })
        }),
    }
}

fn holds32(compare: Compare, x: u32, y: u32) -> bool {
    match compare {
        Compare::Equal => x == y,
        Compare::Greater => (x as i32) > (y as i32),
        Compare::LogicalGreater => x > y,
        Compare::FloatEqual => float_key(x) == float_key(y),
        Compare::FloatGreater => float_key(x) > float_key(y),
        Compare::MagnitudeEqual => float_key(x & 0x7FFF_FFFF) == float_key(y & 0x7FFF_FFFF),
        Compare::MagnitudeGreater => float_key(x & 0x7FFF_FFFF) > float_key(y & 0x7FFF_FFFF),
    }
}

/// `value` rotated left by `bits` as one 128-bit quantity.
pub(super) fn rotate_left_128(value: [u8; 16], bits: u32) -> [u8; 16] {
    u128::from_be_bytes(value).rotate_left(bits).to_be_bytes()
}

/// The distance in ULPs between two single-precision words: the number of
/// values between them in the SPU order, where every zero-exponent word is
/// zero.
///
/// [Schkufza2014 p:58 s:5.2] The distance counts the floating-point numbers
/// between two values.
pub fn ulp_distance(left: u32, right: u32) -> u32 {
    float_key(left).abs_diff(float_key(right)) as u32
}

/// The exponent field of a single-precision word.
pub(super) fn exponent(bits: u32) -> u32 {
    (bits >> 23) & 0xFF
}

/// A nonzero single-precision magnitude as `significand * 2^scale`, the
/// significand with its hidden bit; `None` for a zero exponent.
///
/// [SPU-ISA p:196 s:9] The extended range reads exponent 255 as an ordinary
/// exponent, and a zero exponent as zero.
fn parts(bits: u32) -> Option<(u64, i32)> {
    let biased = exponent(bits);
    (biased != 0).then(|| {
        (
            u64::from(bits & 0x7F_FFFF | 0x80_0000),
            biased as i32 - 127 - 23,
        )
    })
}

/// The magnitude word with `significand` (24 bits, hidden bit set) and
/// `scale`; `None` outside the exponent range 1..=255.
fn word(significand: u64, scale: i32) -> Option<u32> {
    let biased = scale + 127 + 23;
    ((1..=255).contains(&biased))
        .then_some((biased as u32) << 23 | (significand as u32 & 0x7F_FFFF))
}

/// The truncated reciprocal of `x`: the magnitude `Y` with `x * Y < 1` and
/// `x * INC(Y) >= 1`, signed as `x`; `None` for a zero exponent or a `Y`
/// outside the exponent range.
///
/// [SPU-ISA p:216 s:9 Frest] `1/x = Y where x * Y < 1.0 and x * INC(Y) >= 1.0`.
/// With `x = m * 2^s`, the largest significand `n` with `m * n < 2^47` is
/// `(2^47 - 1) / m`, which lies in `[2^23, 2^24)` for every 24-bit `m`.
pub(super) fn truncated_reciprocal(x: u32) -> Option<u32> {
    let (m, s) = parts(x)?;
    let n = ((1u64 << 47) - 1) / m;
    Some(word(n, -s - 47)? | x & 0x8000_0000)
}

/// The truncated reciprocal square root of `|x|`: the magnitude `Y` with
/// `x * Y^2 < 1` and `x * INC(Y)^2 >= 1`.
///
/// [SPU-ISA p:218 s:9 Frsqest] `1/sqrt(x) = Y where x * Y^2 < 1.0 and
/// x * INC(Y)^2 >= 1.0`. With `x = m * 2^s` and `Y = n * 2^e`, the
/// largest `n` with `m * n^2 < 2^t` is the integer square root of
/// `(2^t - 1) / m`, for the `t` of the parity of `s` that puts `n` in
/// `[2^23, 2^24)`.
pub(super) fn truncated_rsqrt(x: u32) -> Option<u32> {
    let (m, s) = parts(x)?;
    let t: i32 = if s.rem_euclid(2) == 0 { 70 } else { 71 };
    let n = (((1u128 << t) - 1) / u128::from(m)).isqrt() as u64;
    let (n, t) = if n >= 1 << 24 {
        (
            (((1u128 << (t - 2)) - 1) / u128::from(m)).isqrt() as u64,
            t - 2,
        )
    } else {
        (n, t)
    };
    word(n, (-s - t) / 2)
}

/// The order a host gives single-precision words: sign and magnitude, a
/// denormal a nonzero value below every normal.
pub(super) fn ieee_order(bits: u32) -> i64 {
    let magnitude = i64::from(bits & 0x7FFF_FFFF);
    if bits & 0x8000_0000 != 0 {
        -magnitude
    } else {
        magnitude
    }
}

/// `significand * 2^scale`, rounded to 24 bits to nearest with ties to
/// even. `significand` carries `guard` extra low bits and `sticky` is true
/// when any bit below them was nonzero.
fn round_nearest(significand: u64, guard: u32, sticky: bool, scale: i32) -> Option<u32> {
    let kept = significand >> guard;
    let dropped = significand & ((1 << guard) - 1);
    let half = 1 << (guard - 1);
    let up = dropped > half || (dropped == half && (sticky || kept & 1 == 1));
    let (kept, scale) = match kept + u64::from(up) {
        overflow if overflow == 1 << 24 => (1 << 23, scale + guard as i32 + 1),
        rounded => (rounded, scale + guard as i32),
    };
    word(kept, scale)
}

/// The host's IEEE `a / b` for normal operands and a normal quotient,
/// rounded to nearest.
pub(super) fn ieee_divide(a: u32, b: u32) -> Option<u32> {
    let ((ma, sa), (mb, sb)) = (parts(a)?, parts(b)?);
    let numerator = ma << 26;
    let quotient = numerator / mb;
    let sticky = numerator % mb != 0;
    // The quotient has 26 or 27 bits; keep 24 and round the rest.
    let guard = 64 - quotient.leading_zeros() - 24;
    let magnitude = round_nearest(quotient, guard, sticky, sa - sb - 26)?;
    Some(magnitude | (a ^ b) & 0x8000_0000)
}

/// The host's IEEE `sqrt(|x|)` for a normal `x`, rounded to nearest.
pub(super) fn ieee_sqrt(x: u32) -> Option<u32> {
    let (m, s) = parts(x)?;
    // An even scale halves exactly.
    let (m, s) = if s.rem_euclid(2) == 0 {
        (m, s)
    } else {
        (m << 1, s - 1)
    };
    let radicand = u128::from(m) << 26;
    let root = radicand.isqrt() as u64;
    let sticky = u128::from(root) * u128::from(root) != radicand;
    let guard = 64 - root.leading_zeros() - 24;
    round_nearest(root, guard, sticky, s / 2 - 13)
}
