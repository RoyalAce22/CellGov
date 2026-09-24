//! Operand-field arithmetic over a 32-bit instruction word, shared by the
//! PPU and SPU fuzz descriptors.
//!
//! A field is a mask over the word. Its value is the masked bits packed
//! low, in bit order, so a field need not be contiguous.

use std::collections::BTreeSet;

/// The low `bits` bits set; all 32 when `bits` is 32 or more.
pub fn low_mask(bits: u32) -> u32 {
    1u32.checked_shl(bits).map_or(u32::MAX, |limit| limit - 1)
}

/// The bits of `word` under `mask`, packed low in bit order.
pub fn extract_bits(word: u32, mask: u32) -> u32 {
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

/// The low bits of `value` spread over the bits of `mask`, in bit order.
pub fn deposit_bits(value: u32, mask: u32) -> u32 {
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

/// The classes an operand field can carry that the shared helpers act on.
pub trait OperandClass: Copy + PartialEq {
    /// The class of a field that selects a register.
    const REGISTER: Self;
    /// The class of a field that carries an immediate.
    const IMMEDIATE: Self;
}

/// One packed operand field in an instruction encoding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OperandField<C> {
    /// Semantic operand class.
    pub class: C,
    /// Bits occupied by the field in the instruction word.
    pub mask: u32,
}

impl<C> OperandField<C> {
    /// Returns the largest unpacked value this field accepts.
    pub fn maximum(&self) -> u32 {
        low_mask(self.mask.count_ones())
    }

    /// Returns values at important signed and unsigned boundaries.
    // [Jiang2022 p:5 s:3.1.1] The maximum and the minimum are the two boundary values an immediate must cover.
    pub fn boundary_values(&self) -> Vec<u32> {
        let maximum = self.maximum();
        let sign = 1u32
            .checked_shl(self.mask.count_ones().saturating_sub(1))
            .unwrap_or(0);
        let mut values = vec![0, 1.min(maximum), sign.saturating_sub(1), sign, maximum];
        values.sort_unstable();
        values.dedup();
        values
    }
}

/// Why operand values do not pack into a descriptor's fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OperandPackError {
    /// The values are not one per field.
    Count {
        /// Field count.
        expected: usize,
        /// Value count.
        found: usize,
    },
    /// A value exceeds its field's maximum.
    OutOfRange,
}

/// Packs one value per field over `canonical_word`, without consulting a
/// decoder.
///
/// # Errors
///
/// [`OperandPackError::Count`] when the value count differs from the
/// field count, then [`OperandPackError::OutOfRange`] when a value
/// exceeds its field's maximum.
pub fn pack_operands<C>(
    canonical_word: u32,
    operands: &[OperandField<C>],
    values: &[u32],
) -> Result<u32, OperandPackError> {
    if values.len() != operands.len() {
        return Err(OperandPackError::Count {
            expected: operands.len(),
            found: values.len(),
        });
    }
    if values
        .iter()
        .zip(operands)
        .any(|(value, field)| *value > field.maximum())
    {
        return Err(OperandPackError::OutOfRange);
    }
    let mut word = canonical_word;
    for (field, value) in operands.iter().zip(values) {
        word = (word & !field.mask) | deposit_bits(*value, field.mask);
    }
    Ok(word)
}

/// The value of each field in `canonical_word`, in field order.
pub fn canonical_parameters<C>(canonical_word: u32, operands: &[OperandField<C>]) -> Vec<u32> {
    operands
        .iter()
        .map(|field| extract_bits(canonical_word, field.mask))
        .collect()
}

/// One word per immediate boundary that `encode` accepts, each from the
/// canonical values with one immediate at a boundary, in ascending order.
pub fn immediate_boundary_words<C: OperandClass>(
    canonical_word: u32,
    operands: &[OperandField<C>],
    mut encode: impl FnMut(&[u32]) -> Option<u32>,
) -> Vec<u32> {
    let canonical = canonical_parameters(canonical_word, operands);
    let mut words = BTreeSet::new();
    for (index, field) in operands.iter().enumerate() {
        if field.class != C::IMMEDIATE {
            continue;
        }
        for value in field.boundary_values() {
            let mut parameters = canonical.clone();
            parameters[index] = value;
            if let Some(word) = encode(&parameters) {
                words.insert(word);
            }
        }
    }
    words.into_iter().collect()
}

/// The canonical values with every register field set to `value`, or
/// `None` when no field selects a register.
pub fn register_alias_parameters<C: OperandClass>(
    canonical_word: u32,
    operands: &[OperandField<C>],
    value: u32,
) -> Option<Vec<u32>> {
    let mut parameters = canonical_parameters(canonical_word, operands);
    let mut changed = false;
    for (parameter, field) in parameters.iter_mut().zip(operands) {
        if field.class == C::REGISTER {
            *parameter = value & field.maximum();
            changed = true;
        }
    }
    changed.then_some(parameters)
}

/// The union of every field's mask.
pub fn operand_mask<C>(operands: &[OperandField<C>]) -> u32 {
    operands.iter().fold(0, |mask, field| mask | field.mask)
}

/// `raw` with one set operand bit cleared, for each set operand bit, in
/// field order and then bit order.
pub fn operand_bit_clears<C>(raw: u32, operands: &[OperandField<C>]) -> Vec<u32> {
    operands
        .iter()
        .flat_map(|field| (0..u32::BITS).map(move |bit| (field, bit)))
        .filter_map(|(field, bit)| {
            let bit_mask = 1u32 << bit;
            (field.mask & bit_mask != 0 && raw & bit_mask != 0).then_some(raw & !bit_mask)
        })
        .collect()
}

#[cfg(test)]
#[path = "tests/operand_tests.rs"]
mod tests;
