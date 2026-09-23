//! Descriptor-driven generation of SPU words, parameters and sequences, and
//! the initial state a case runs from.

use std::collections::BTreeSet;

use cellgov_spu::fuzz::{
    generation_descriptors, SpuGenerationDescriptor, SpuGenerationError, SpuOperandClass,
    SpuSequenceFlow, SpuSequenceInteraction, SpuStateInput,
};
use cellgov_spu::state::{SpuState, SPU_LS_SIZE};
use cellgov_sync::{ReservedLine, RESERVATION_LINE_BYTES};

use crate::boundary::call_target;
use crate::case::CaseFeature;
use crate::error::{FuzzError, GeneratorError, InvariantError};
use crate::rng::Rng;
use crate::{FuzzConfig, GenerationStrategy, ParameterStream};

const STRUCTURED_ENCODING_ATTEMPTS: usize = 64;
pub(super) const STRUCTURED_LS_DATA_BASE: u32 = 0x1_0000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct GeneratedWord {
    pub(super) raw: u32,
    pub(super) features: BTreeSet<CaseFeature>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct GeneratedSequence {
    pub(super) words: Vec<u32>,
    pub(super) features: BTreeSet<CaseFeature>,
    pub(super) interaction: Option<(SpuSequenceInteraction, u32)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct GeneratedParameters {
    stream: ParameterStream,
    features: BTreeSet<CaseFeature>,
}

pub(super) fn case_descriptors(
    config: FuzzConfig,
) -> Result<Vec<SpuGenerationDescriptor>, FuzzError> {
    match config.strategy {
        GenerationStrategy::Structured => call_target(generation_descriptors).map_err(|_| {
            InvariantError::UnexpectedPanic {
                stage: "SPU descriptor registry",
            }
            .into()
        }),
        GenerationStrategy::RawWords => Ok(Vec::new()),
    }
}

pub(super) fn structured_sequence(
    descriptors: &[SpuGenerationDescriptor],
    rng: &mut Rng,
    count: usize,
) -> Result<GeneratedSequence, GeneratorError> {
    // [Wang2024 p:340:1 s:Abstract] Generated programs track program state statically.
    if count >= 3 && rng.chance(1, 8)? {
        // [Padhye2019 p:329 s:Abstract] Structural parameter mutation retains valid inputs.
        // [Feng2026 p:25 s:Abstract] Paired runs compare normalized fault outcomes.
        let choices = SpuSequenceInteraction::ALL.len();
        let index = rng.below(choices as u64)? as usize;
        let interaction = SpuSequenceInteraction::ALL[index];
        let data_base =
            STRUCTURED_LS_DATA_BASE + (rng.below(16)? as u32 * RESERVATION_LINE_BYTES as u32);
        let mut words = interaction.words(descriptors)?;
        let nop = descriptors
            .iter()
            .find(|descriptor| descriptor.kind == cellgov_spu::instruction::SpuInstructionKind::Nop)
            .ok_or(SpuGenerationError::MissingSequenceKind {
                kind: cellgov_spu::instruction::SpuInstructionKind::Nop,
            })?
            .canonical_word;
        words.resize(count, nop);
        let features = match interaction {
            SpuSequenceInteraction::Branch | SpuSequenceInteraction::Stop => {
                BTreeSet::from([CaseFeature::ControlledFlow])
            }
            SpuSequenceInteraction::LocalStore => BTreeSet::from([CaseFeature::MappedMemory]),
            SpuSequenceInteraction::LocalStoreFault => {
                BTreeSet::from([CaseFeature::ChannelState, CaseFeature::NamedFaultBoundary])
            }
            SpuSequenceInteraction::Channel
            | SpuSequenceInteraction::Mailbox
            | SpuSequenceInteraction::Dma
            | SpuSequenceInteraction::DmaGet
            | SpuSequenceInteraction::MemoryRead
            | SpuSequenceInteraction::Reservation => BTreeSet::from([CaseFeature::ChannelState]),
        };
        return Ok(GeneratedSequence {
            words,
            features,
            interaction: Some((interaction, data_base)),
        });
    }
    let chain_register = rng.chance(3, 4)?.then(|| rng.next_u32());
    let mut words = Vec::with_capacity(count);
    let mut features = BTreeSet::new();
    let mut dependency_chain = chain_register.is_some() && count > 1;
    for index in 0..count {
        let linear_only = index + 1 < count;
        let generated =
            structured_generated_word_for_flow(descriptors, rng, chain_register, linear_only)?;
        dependency_chain &= generated.features.contains(&CaseFeature::OperandAlias);
        words.push(generated.raw);
        features.extend(generated.features);
    }
    if dependency_chain {
        features.insert(CaseFeature::DependencyChain);
    }
    Ok(GeneratedSequence {
        words,
        features,
        interaction: None,
    })
}

fn structured_generated_word_for_flow(
    descriptors: &[SpuGenerationDescriptor],
    rng: &mut Rng,
    forced_alias: Option<u32>,
    linear_only: bool,
) -> Result<GeneratedWord, GeneratorError> {
    for _ in 0..STRUCTURED_ENCODING_ATTEMPTS {
        if descriptors.is_empty() {
            return Err(GeneratorError::EmptyDescriptorRegistry { target: "SPU" });
        }
        let index = rng.below(descriptors.len() as u64)? as usize;
        let Some(descriptor) = descriptors.get(index) else {
            return Err(GeneratorError::EmptyDescriptorRegistry { target: "SPU" });
        };
        if descriptor.sequence_flow == SpuSequenceFlow::Linear
            || (!linear_only && descriptor.sequence_flow == SpuSequenceFlow::ControlTransfer)
        {
            let mut generated =
                match structured_generated_word_for_descriptor(descriptor, rng, forced_alias) {
                    Ok(generated) => generated,
                    Err(GeneratorError::ConstraintAttemptsExhausted { .. }) => continue,
                    Err(error) => return Err(error),
                };
            if descriptor.sequence_flow == SpuSequenceFlow::ControlTransfer {
                generated.features.insert(CaseFeature::ControlledFlow);
            }
            return Ok(generated);
        }
    }
    Err(GeneratorError::ConstraintAttemptsExhausted {
        target: "SPU sequence descriptor",
        attempts: STRUCTURED_ENCODING_ATTEMPTS,
    })
}

pub(super) fn structured_generated_word(
    descriptors: &[SpuGenerationDescriptor],
    rng: &mut Rng,
    forced_alias: Option<u32>,
) -> Result<GeneratedWord, GeneratorError> {
    if descriptors.is_empty() {
        return Err(GeneratorError::EmptyDescriptorRegistry { target: "SPU" });
    }
    let index = rng.below(descriptors.len() as u64)? as usize;
    let Some(descriptor) = descriptors.get(index) else {
        return Err(GeneratorError::EmptyDescriptorRegistry { target: "SPU" });
    };
    structured_generated_word_for_descriptor(descriptor, rng, forced_alias)
}

fn structured_generated_word_for_descriptor(
    descriptor: &SpuGenerationDescriptor,
    rng: &mut Rng,
    forced_alias: Option<u32>,
) -> Result<GeneratedWord, GeneratorError> {
    // [Yang2011 p:1 s:Abstract] Only valid typed operand combinations reach comparison.
    for _ in 0..STRUCTURED_ENCODING_ATTEMPTS {
        let parameters = generated_spu_parameters(descriptor, rng, forced_alias)?;
        match descriptor.encode(parameters.stream.values()) {
            Ok(raw) => {
                return Ok(GeneratedWord {
                    raw,
                    features: parameters.features,
                })
            }
            Err(SpuGenerationError::InvalidOperands) => {}
            Err(error) => return Err(error.into()),
        }
    }
    Err(GeneratorError::ConstraintAttemptsExhausted {
        target: "SPU",
        attempts: STRUCTURED_ENCODING_ATTEMPTS,
    })
}

fn generated_spu_parameters(
    descriptor: &SpuGenerationDescriptor,
    rng: &mut Rng,
    forced_alias: Option<u32>,
) -> Result<GeneratedParameters, GeneratorError> {
    let alias = match forced_alias {
        Some(value) => Some(value),
        None => rng.chance(1, 8)?.then(|| rng.next_u32()),
    };
    let register_fields = descriptor
        .operands
        .iter()
        .filter(|field| field.class == SpuOperandClass::Register)
        .count();
    let mut features = BTreeSet::new();
    if alias.is_some() && register_fields > 1 {
        features.insert(CaseFeature::OperandAlias);
    }
    let mut values = Vec::with_capacity(descriptor.operands.len());
    for field in &descriptor.operands {
        let value = if field.class == SpuOperandClass::Channel
            && !descriptor.channel_values.is_empty()
            && rng.chance(3, 4)?
        {
            let channel_index = rng.below(descriptor.channel_values.len() as u64)? as usize;
            descriptor
                .channel_values
                .get(channel_index)
                .copied()
                .ok_or(GeneratorError::EmptyDescriptorRegistry {
                    target: "SPU channel values",
                })?
        } else if field.class == SpuOperandClass::Register && alias.is_some() {
            alias.unwrap_or(0) & field.maximum()
        } else if rng.chance(1, 4)? {
            features.insert(CaseFeature::OperandBoundary);
            let boundaries = field.boundary_values();
            let boundary_index = rng.below(boundaries.len() as u64)? as usize;
            boundaries.get(boundary_index).copied().ok_or(
                GeneratorError::EmptyDescriptorRegistry {
                    target: "SPU operand boundaries",
                },
            )?
        } else {
            rng.next_u32() & field.maximum()
        };
        values.push(value);
    }
    Ok(GeneratedParameters {
        stream: ParameterStream::new(values),
        features,
    })
}

pub(super) fn random_state(rng: &mut Rng) -> Result<SpuState, FuzzError> {
    let mut state = SpuState::new();
    for register in &mut state.regs {
        rng.fill(register);
    }
    rng.fill(&mut state.ls);
    state.pc = pc_for_slot(rng.below((SPU_LS_SIZE / 4) as u64)?)?;
    state.channels.mfc_lsa = rng.next_u32();
    state.channels.mfc_eah = rng.next_u32();
    state.channels.mfc_eal = rng.next_u32();
    state.channels.mfc_size = rng.next_u32();
    state.channels.mfc_tag_id = rng.next_u32();
    state.channels.tag_mask = rng.next_u32();
    state.channels.tag_status = rng.next_u32();
    state.channels.atomic_status = rng.next_u32();
    state.channels.pending_mbox_rt = None;
    state.channels.pending_get = None;
    state.reservation = if rng.chance(1, 2)? {
        Some(ReservedLine::containing(
            rng.next_u64() & ((1u64 << 42) - 1),
        ))
    } else {
        None
    };
    Ok(state)
}

pub(super) fn state_aware_state(
    rng: &mut Rng,
    input: Option<SpuStateInput>,
) -> Result<(SpuState, BTreeSet<CaseFeature>), FuzzError> {
    let mut state = random_state(rng)?;
    // Register addresses must stay after the generated program and inside local store.
    // Indexed forms add two register values.
    for register in 0u8..128 {
        let offset = u32::from(register % 16) * 16;
        state.set_reg_word_splat(register, STRUCTURED_LS_DATA_BASE + offset);
    }
    state.channels.mfc_lsa = STRUCTURED_LS_DATA_BASE;
    state.channels.mfc_eah = 0;
    state.channels.mfc_eal = STRUCTURED_LS_DATA_BASE;
    state.channels.mfc_size = RESERVATION_LINE_BYTES as u32;
    state.channels.mfc_tag_id = 0;
    state.channels.tag_mask = 1;
    state.channels.tag_status = 1;
    state.channels.atomic_status = 0;
    state.channels.pending_mbox_rt = None;
    state.channels.pending_get = None;
    state.reservation = Some(ReservedLine::containing(u64::from(STRUCTURED_LS_DATA_BASE)));
    // [Wang2024 p:340:1 s:Abstract] Generated programs track program state statically.
    if let Some(input) = input {
        let value = if input.preferred.is_some() && rng.chance(1, 2)? {
            input.preferred.unwrap_or(0)
        } else {
            let index = rng.below(input.values.len() as u64)? as usize;
            input
                .values
                .get(index)
                .copied()
                .ok_or(GeneratorError::EmptyDescriptorRegistry {
                    target: "SPU state input values",
                })?
        };
        state.set_reg_word_splat(input.register, value);
    }
    Ok((
        state,
        BTreeSet::from([
            CaseFeature::MappedMemory,
            CaseFeature::Reservation,
            CaseFeature::ChannelState,
        ]),
    ))
}

fn pc_for_slot(slot: u64) -> Result<u32, InvariantError> {
    let limit = (SPU_LS_SIZE / 4) as u64;
    if slot >= limit {
        return Err(InvariantError::ValueOutOfRange {
            value_kind: "SPU instruction slot",
            value: slot,
        });
    }
    u32::try_from(slot * 4).map_err(|_| InvariantError::ValueOutOfRange {
        value_kind: "SPU program counter",
        value: slot * 4,
    })
}

#[cfg(test)]
#[path = "tests/generate_tests.rs"]
mod tests;
