//! The single-bit simplifiers.

use crate::instruction::SpuInstructionKind;

use super::registry::generation_descriptor;

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
