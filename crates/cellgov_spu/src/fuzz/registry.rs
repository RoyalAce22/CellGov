//! The registry of generation descriptors, one per decoded instruction kind.

use std::collections::{BTreeMap, BTreeSet};

use strum::VariantArray;

use crate::instruction::SpuInstructionKind;

use super::classify::sequence_flow;
use super::fields::operand_fields;
use super::support::channel_values;
use super::types::SpuGenerationDescriptor;

/// Lists every SPU instruction recipe in kind order.
pub fn generation_descriptors() -> Vec<SpuGenerationDescriptor> {
    build_generation_descriptors()
}

/// Finds the generation recipe for a decoded word.
pub fn generation_descriptor(raw: u32) -> Option<SpuGenerationDescriptor> {
    let instruction = crate::decode::decode(raw).ok()?;
    let contract = instruction.fuzz_descriptor();
    Some(SpuGenerationDescriptor {
        kind: contract.kind,
        form: contract.form,
        sequence_flow: sequence_flow(contract.kind, contract.outcomes),
        channel_values: channel_values(contract.kind),
        canonical_word: raw,
        operands: operand_fields(raw, instruction, contract),
    })
}

fn build_generation_descriptors() -> Vec<SpuGenerationDescriptor> {
    let mut words = BTreeMap::new();
    // Scanning every upper 18-bit value covers each opcode family.
    // Zero in the low 14 bits supplies canonical register values.
    for upper in 0..(1u32 << 18) {
        let raw = upper << 14;
        let Ok(instruction) = crate::decode::decode(raw) else {
            continue;
        };
        words
            .entry(SpuInstructionKind::from(instruction))
            .or_insert(raw);
    }
    words
        .into_iter()
        .filter_map(|(kind, canonical_word)| {
            let instruction = crate::decode::decode(canonical_word).ok()?;
            let contract = instruction.fuzz_descriptor();
            Some(SpuGenerationDescriptor {
                kind,
                form: contract.form,
                sequence_flow: sequence_flow(kind, contract.outcomes),
                channel_values: channel_values(kind),
                canonical_word,
                operands: operand_fields(canonical_word, instruction, contract),
            })
        })
        .collect()
}

/// Lists every decoded SPU kind without consulting generator recipes.
pub fn expected_generation_kinds() -> BTreeSet<SpuInstructionKind> {
    SpuInstructionKind::VARIANTS.iter().copied().collect()
}
