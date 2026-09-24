//! The single-bit simplifiers.

use super::registry::generation_descriptor;

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
