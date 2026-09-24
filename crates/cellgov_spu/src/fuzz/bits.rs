//! Bit extraction and deposit over masks, and the single-bit simplifiers.

use crate::instruction::SpuInstructionKind;

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

/// Produces a simpler encoding when it keeps the instruction kind.
pub fn simplify_instruction_bit(raw: u32, bit_index: u32) -> Option<u32> {
    let bit = 1u32.checked_shl(bit_index)?;
    (raw & bit != 0).then_some(())?;
    let instruction = crate::decode::decode(raw).ok()?;
    let kind = SpuInstructionKind::from(instruction);
    let candidate = raw & !bit;
    let decoded = crate::decode::decode(candidate).ok()?;
    (SpuInstructionKind::from(decoded) == kind).then_some(candidate)
}

/// Produces valid same-kind candidates for fuzz-engine shrinking.
pub fn shrink_instruction(raw: u32) -> Vec<u32> {
    generation_descriptor(raw).map_or_else(Vec::new, |descriptor| descriptor.shrink(raw))
}
