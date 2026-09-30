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
