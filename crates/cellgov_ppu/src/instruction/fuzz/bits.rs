//! Bit extraction and deposit over masks, and the single-bit simplifiers.

use super::registry::generation_descriptor;

pub(super) fn extract_bits(word: u32, mask: u32) -> u32 {
    let mut packed = 0;
    let mut destination = 0;
    for source in 0..u32::BITS {
        let bit = 1u32 << source;
        if mask & bit != 0 {
            if word & bit != 0 {
                packed |= 1u32 << destination;
            }
            destination += 1;
        }
    }
    packed
}

pub(super) fn deposit_bits(value: u32, mask: u32) -> u32 {
    let mut deposited = 0;
    let mut source = 0;
    for destination in 0..u32::BITS {
        let bit = 1u32 << destination;
        if mask & bit != 0 {
            if value & (1u32 << source) != 0 {
                deposited |= bit;
            }
            source += 1;
        }
    }
    deposited
}

pub(super) fn low_mask(bits: u32) -> u32 {
    1u32.checked_shl(bits).map_or(u32::MAX, |limit| limit - 1)
}

/// Clear one raw bit and retain only a decodable encoding of the exact same kind.
pub fn simplify_bit(raw: u32, bit: u8) -> Option<u32> {
    if bit >= u32::BITS as u8 || raw & (1u32 << bit) == 0 {
        return None;
    }
    let instruction = crate::decode::decode(raw).ok()?;
    let kind = instruction.fuzz_descriptor(raw).kind;
    let candidate = raw & !(1u32 << bit);
    let decoded = crate::decode::decode(candidate).ok()?;
    (decoded.fuzz_descriptor(candidate).kind == kind).then_some(candidate)
}

/// Produces valid exact-kind encodings by clearing one set operand bit.
pub fn simplify_encoding(raw: u32) -> Vec<u32> {
    generation_descriptor(raw).map_or_else(Vec::new, |descriptor| descriptor.shrink(raw))
}
