//! Encoding, packing and shrinking through a generation descriptor.

use std::collections::BTreeSet;

use super::bits::{deposit_bits, extract_bits};
use super::classify::exact_kind;
use super::fields::operand_combination_is_valid;
use super::types::{SpuGenerationDescriptor, SpuGenerationError, SpuOperandClass};

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
        if values.len() != self.operands.len() {
            return Err(SpuGenerationError::OperandCount {
                expected: self.operands.len(),
                found: values.len(),
            });
        }
        if values
            .iter()
            .zip(&self.operands)
            .any(|(value, field)| *value > field.maximum())
        {
            return Err(SpuGenerationError::InvalidOperands);
        }
        let mut word = self.canonical_word;
        for (field, value) in self.operands.iter().zip(values) {
            word = (word & !field.mask) | deposit_bits(*value, field.mask);
        }
        Ok(word)
    }

    /// Checks documented operand combinations without consulting the decoder.
    pub fn operands_are_defined(&self, word: u32) -> bool {
        operand_combination_is_valid(self.kind, word)
    }

    /// Reads the canonical values in operand order.
    pub fn canonical_parameters(&self) -> Vec<u32> {
        self.operands
            .iter()
            .map(|field| extract_bits(self.canonical_word, field.mask))
            .collect()
    }

    /// Produces one valid witness for each immediate boundary.
    pub fn immediate_boundary_words(&self) -> Vec<u32> {
        let canonical = self.canonical_parameters();
        let mut words = BTreeSet::new();
        for (index, field) in self.operands.iter().enumerate() {
            if field.class != SpuOperandClass::Immediate {
                continue;
            }
            for value in field.boundary_values() {
                let mut parameters = canonical.clone();
                parameters[index] = value;
                if let Ok(word) = self.encode(&parameters) {
                    words.insert(word);
                }
            }
        }
        words.into_iter().collect()
    }

    /// Produces a valid word whose register operands share one encoded value.
    pub fn alias_word(&self, value: u32) -> Option<u32> {
        let mut parameters = self.canonical_parameters();
        let mut changed = false;
        for (parameter, field) in parameters.iter_mut().zip(&self.operands) {
            if field.class == SpuOperandClass::Register {
                *parameter = value & field.maximum();
                changed = true;
            }
        }
        changed.then(|| self.encode(&parameters).ok()).flatten()
    }

    /// Produces same-kind words by toggling non-operand bits.
    pub fn reserved_bit_words(&self) -> Vec<u32> {
        let original = crate::decode::decode(self.canonical_word).ok();
        let operand_mask = self
            .operands
            .iter()
            .fold(0, |mask, field| mask | field.mask);
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
        self.operands
            .iter()
            .flat_map(|field| (0..u32::BITS).map(move |bit| (field, bit)))
            .filter_map(|(field, bit)| {
                let bit_mask = 1u32 << bit;
                if field.mask & bit_mask == 0 || raw & bit_mask == 0 {
                    return None;
                }
                let candidate = raw & !bit_mask;
                (exact_kind(candidate) == Some(self.kind)).then_some(candidate)
            })
            .collect()
    }
}
