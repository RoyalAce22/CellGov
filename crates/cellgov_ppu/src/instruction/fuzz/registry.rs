//! The registry of generation descriptors, one per exact fuzz kind.

use std::collections::{BTreeMap, BTreeSet};

use strum::{IntoEnumIterator, VariantArray};

use crate::instruction::ops::{Fp59Op, Fp63Op, VaOp, VxOp};
use crate::instruction::PpuInstructionKind;

use super::classify::{sequence_class, sequence_dependency, sequence_flow};
use super::fields::{generation_operands_are_valid, operand_fields};
use super::types::{PpuEncodingForm, PpuFuzzKind, PpuGenerationDescriptor};

/// Lists every standalone PPU instruction recipe in exact-kind order.
pub fn generation_descriptors() -> Vec<PpuGenerationDescriptor> {
    build_generation_descriptors()
}

/// Lists every standalone decoded kind without consulting generator recipes.
pub fn expected_generation_kinds() -> BTreeSet<PpuFuzzKind> {
    let mut expected = BTreeSet::new();
    for kind in PpuInstructionKind::VARIANTS {
        if !is_synthetic_kind(*kind)
            && !matches!(
                kind,
                PpuInstructionKind::Vx
                    | PpuInstructionKind::Va
                    | PpuInstructionKind::Fp59
                    | PpuInstructionKind::Fp63
            )
        {
            expected.insert(PpuFuzzKind::Ordinary(*kind));
        }
    }
    expected.extend(
        VxOp::iter()
            .filter(|op| *op != VxOp::Vxor)
            .map(PpuFuzzKind::Vx),
    );
    expected.extend(
        VaOp::iter()
            .filter(|op| *op != VaOp::Vsldoi)
            .map(PpuFuzzKind::Va),
    );
    expected.extend(Fp59Op::iter().map(PpuFuzzKind::Fp59));
    expected.extend(Fp63Op::iter().map(PpuFuzzKind::Fp63));
    expected
}

fn is_synthetic_kind(kind: PpuInstructionKind) -> bool {
    use PpuInstructionKind as K;
    matches!(
        kind,
        K::Li
            | K::Mr
            | K::Slwi
            | K::Srwi
            | K::Clrlwi
            | K::Nop
            | K::CmpwZero
            | K::Clrldi
            | K::Sldi
            | K::Srdi
            | K::LwzCmpwi
            | K::LiStw
            | K::MflrStw
            | K::LwzMtlr
            | K::MflrStd
            | K::LdMtlr
            | K::StdStd
            | K::CmpwiBc
            | K::CmpwBc
            | K::Consumed
    )
}

/// Finds the generation recipe for a decoded word.
pub fn generation_descriptor(raw: u32) -> Option<PpuGenerationDescriptor> {
    let instruction = crate::decode::decode(raw).ok()?;
    let contract = instruction.fuzz_descriptor(raw);
    (contract.form != PpuEncodingForm::Synthetic).then(|| PpuGenerationDescriptor {
        kind: contract.kind,
        form: contract.form,
        sequence_class: sequence_class(contract.kind),
        sequence_flow: sequence_flow(contract.outcomes),
        sequence_dependency: sequence_dependency(contract.kind),
        canonical_word: raw,
        operands: operand_fields(raw, instruction, contract),
    })
}

fn build_generation_descriptors() -> Vec<PpuGenerationDescriptor> {
    let mut words = BTreeMap::new();

    // Decoder discriminators use:
    // - the primary opcode;
    // - the low 11 bits;
    // - the two middle five-bit fields in XFX forms.
    // The finite scan does not sample operand data.
    let middle_templates = [
        0,
        1 << 11,
        31 << 11,
        1 << 16,
        31 << 16,
        1 << 21,
        31 << 21,
        (3 << 11) | (4 << 16) | (5 << 21),
    ];
    for primary in 0..64u32 {
        for suffix in 0..2048u32 {
            for middle in middle_templates {
                retain_generation_word((primary << 26) | middle | suffix, &mut words);
            }
        }
    }
    for selector in 0..1024u32 {
        for suffix in 0..2048u32 {
            retain_generation_word((31 << 26) | (selector << 11) | suffix, &mut words);
        }
    }

    // The family enums define the exact operations independently of the raw-word scan.
    for op in VxOp::iter() {
        retain_generation_word((4 << 26) | u32::from(op as u16), &mut words);
    }
    for op in VaOp::iter() {
        retain_generation_word((4 << 26) | u32::from(op as u8), &mut words);
    }
    for op in Fp59Op::iter() {
        retain_generation_word((59 << 26) | (u32::from(op as u16) << 1), &mut words);
    }
    for op in Fp63Op::iter() {
        retain_generation_word((63 << 26) | (u32::from(op as u16) << 1), &mut words);
    }

    words
        .into_iter()
        .filter_map(|(kind, canonical_word)| {
            let instruction = crate::decode::decode(canonical_word).ok()?;
            let contract = instruction.fuzz_descriptor(canonical_word);
            Some(PpuGenerationDescriptor {
                kind,
                form: contract.form,
                sequence_class: sequence_class(kind),
                sequence_flow: sequence_flow(contract.outcomes),
                sequence_dependency: sequence_dependency(kind),
                canonical_word,
                operands: operand_fields(canonical_word, instruction, contract),
            })
        })
        .collect()
}

fn retain_generation_word(raw: u32, words: &mut BTreeMap<PpuFuzzKind, u32>) {
    let Ok(instruction) = crate::decode::decode(raw) else {
        return;
    };
    let descriptor = instruction.fuzz_descriptor(raw);
    if descriptor.form != PpuEncodingForm::Synthetic && generation_operands_are_valid(instruction) {
        let canonical = words.entry(descriptor.kind).or_insert(raw);
        // [PPC-Book1 p:66 s:3.3.11] ori is primary-24 D-form. Decoder
        // aliases such as isync and cache hints must not hide its operands.
        if descriptor.kind == PpuFuzzKind::Ordinary(PpuInstructionKind::Ori) && raw >> 26 == 24 {
            *canonical = raw;
        }
    }
}
