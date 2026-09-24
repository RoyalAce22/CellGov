//! Encoding, packing and shrinking through a generation descriptor.

use cellgov_exec::operand::{self, OperandPackError};

use super::classify::exact_kind;
use super::fields::operand_combination_is_valid;
use super::types::{SpuGenerationDescriptor, SpuGenerationError};

impl SpuGenerationDescriptor {
    /// Encodes one value per typed operand field.
    ///
    /// # Errors
    ///
    /// - If the number of values is incorrect, the method returns [`SpuGenerationError::OperandCount`].
    /// - If the operands are invalid, the method returns [`SpuGenerationError::InvalidOperands`].
    pub fn encode(&self, values: &[u32]) -> Result<u32, SpuGenerationError> {
        let word = self.pack_operands(values)?;
        if !operand_combination_is_valid(self.kind, word) {
            return Err(SpuGenerationError::InvalidOperands);
        }
        exact_kind(word)
            .filter(|kind| *kind == self.kind)
            .map(|_| word)
            .ok_or(SpuGenerationError::InvalidOperands)
    }

    /// Packs in-range descriptor operands without consulting the decoder.
    pub fn pack_operands(&self, values: &[u32]) -> Result<u32, SpuGenerationError> {
        operand::pack_operands(self.canonical_word, &self.operands, values).map_err(|error| {
            match error {
                OperandPackError::Count { expected, found } => {
                    SpuGenerationError::OperandCount { expected, found }
                }
                OperandPackError::OutOfRange => SpuGenerationError::InvalidOperands,
            }
        })
    }

    /// Checks documented operand combinations without consulting the decoder.
    pub fn operands_are_defined(&self, word: u32) -> bool {
        operand_combination_is_valid(self.kind, word)
    }

    /// Reads the canonical values in operand order.
    pub fn canonical_parameters(&self) -> Vec<u32> {
        operand::canonical_parameters(self.canonical_word, &self.operands)
    }

    /// Produces one valid witness for each immediate boundary.
    pub fn immediate_boundary_words(&self) -> Vec<u32> {
        operand::immediate_boundary_words(self.canonical_word, &self.operands, |parameters| {
            self.encode(parameters).ok()
        })
    }

    /// Produces a valid word whose register operands share one encoded value.
    pub fn alias_word(&self, value: u32) -> Option<u32> {
        operand::register_alias_parameters(self.canonical_word, &self.operands, value)
            .and_then(|parameters| self.encode(&parameters).ok())
    }

    /// Produces same-kind words by toggling non-operand bits.
    pub fn reserved_bit_words(&self) -> Vec<u32> {
        let original = crate::decode::decode(self.canonical_word).ok();
        let operand_mask = operand::operand_mask(&self.operands);
        (0..u32::BITS)
            .filter_map(|bit| {
                if operand_mask & (1u32 << bit) != 0 {
                    return None;
                }
                let candidate = self.canonical_word ^ (1u32 << bit);
                (crate::decode::decode(candidate).ok() == original).then_some(candidate)
            })
            .collect()
    }

    /// Produces valid same-kind words by clearing one encoded operand bit.
    // [Regehr2012 p:4 s:5.2] A reducer that emits only variants it knows are valid never chases a difference an invalid variant caused.
    pub fn shrink(&self, raw: u32) -> Vec<u32> {
        operand::operand_bit_clears(raw, &self.operands)
            .into_iter()
            .filter(|&candidate| exact_kind(candidate) == Some(self.kind))
            .collect()
    }
}
