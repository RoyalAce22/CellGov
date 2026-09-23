//! Descriptor-driven generation of PPU words, parameters and sequences, the
//! initial state a case runs from, and the data region that state points into.

use std::collections::BTreeSet;

use cellgov_ppu::instruction::fuzz::{
    generation_descriptors, PpuGenerationDescriptor, PpuGenerationError, PpuOperandClass,
    PpuSequenceClass, PpuSequenceFlow,
};
use cellgov_ppu::instruction::PpuInstruction;
use cellgov_ppu::state::PpuState;
use cellgov_sync::ReservedLine;

use crate::boundary::call_target;
use crate::case::CaseFeature;
use crate::error::{FuzzError, GeneratorError, InvariantError};
use crate::rng::Rng;
use crate::{FuzzConfig, GenerationStrategy, ParameterStream};

pub(super) const DATA_BASE: u64 = 0x1000_0000;
pub(super) const DATA_REGION_BASE: u64 = DATA_BASE - 32 * 1024;
pub(super) const DATA_LEN: usize = 64 * 1024;
const STRUCTURED_ENCODING_ATTEMPTS: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct GeneratedWord {
    pub(super) raw: u32,
    pub(super) features: BTreeSet<CaseFeature>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct GeneratedSequence {
    pub(super) words: Vec<u32>,
    pub(super) features: BTreeSet<CaseFeature>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct GeneratedParameters {
    stream: ParameterStream,
    features: BTreeSet<CaseFeature>,
}

pub(super) fn case_descriptors(
    config: FuzzConfig,
) -> Result<Vec<PpuGenerationDescriptor>, FuzzError> {
    match config.strategy {
        GenerationStrategy::Structured => call_target(generation_descriptors).map_err(|_| {
            InvariantError::UnexpectedPanic {
                stage: "PPU descriptor registry",
            }
            .into()
        }),
        GenerationStrategy::RawWords => Ok(Vec::new()),
    }
}

#[cfg(test)]
fn structured_words(
    descriptors: &[PpuGenerationDescriptor],
    rng: &mut Rng,
    count: usize,
) -> Result<Vec<u32>, GeneratorError> {
    structured_sequence(descriptors, rng, count).map(|generated| generated.words)
}

pub(super) fn structured_sequence(
    descriptors: &[PpuGenerationDescriptor],
    rng: &mut Rng,
    count: usize,
) -> Result<GeneratedSequence, GeneratorError> {
    // [PPC-Book1 p:48 s:3.3] lswx validity depends on the XER byte count.
    // The interpreter replaces that count only through mtxer, so one of the two
    // instruction kinds is omitted deterministically from each generated sequence.
    let excluded = if rng.chance(1, 2)? {
        PpuSequenceClass::ReadsXerByteCount
    } else {
        PpuSequenceClass::ReplacesXer
    };
    // [Wang2024 p:340:9 s:3.2] Generation records which places earlier words wrote so later words can read them.
    let chain_register = rng.chance(3, 4)?.then(|| rng.next_u32());
    let mut words = Vec::with_capacity(count);
    let mut features = BTreeSet::new();
    for index in 0..count {
        let linear_only = index + 1 < count;
        let generated = structured_generated_word_excluding(
            descriptors,
            rng,
            excluded,
            chain_register,
            linear_only,
        )?;
        words.push(generated.raw);
        features.extend(generated.features);
    }
    if chain_register.is_some() && count > 1 {
        features.insert(CaseFeature::DependencyChain);
    }
    Ok(GeneratedSequence { words, features })
}

fn structured_generated_word_excluding(
    descriptors: &[PpuGenerationDescriptor],
    rng: &mut Rng,
    excluded: PpuSequenceClass,
    forced_alias: Option<u32>,
    linear_only: bool,
) -> Result<GeneratedWord, GeneratorError> {
    let eligible = descriptors
        .iter()
        .filter(|descriptor| {
            descriptor.sequence_class != excluded
                && (forced_alias.is_none() || descriptor.sequence_dependency.is_some())
                && (descriptor.sequence_flow == PpuSequenceFlow::Linear
                    || (!linear_only
                        && descriptor.sequence_flow == PpuSequenceFlow::ControlTransfer))
        })
        .collect::<Vec<_>>();
    if eligible.is_empty() {
        return Err(GeneratorError::EmptyDescriptorRegistry {
            target: "PPU sequence",
        });
    }
    for _ in 0..STRUCTURED_ENCODING_ATTEMPTS {
        let index = rng.below(eligible.len() as u64)? as usize;
        let Some(descriptor) = eligible.get(index).copied() else {
            return Err(GeneratorError::EmptyDescriptorRegistry { target: "PPU" });
        };
        let mut generated =
            match structured_generated_word_for_descriptor(descriptor, rng, forced_alias) {
                Ok(generated) => generated,
                Err(GeneratorError::ConstraintAttemptsExhausted { .. }) => continue,
                Err(error) => return Err(error),
            };
        if descriptor.sequence_flow == PpuSequenceFlow::ControlTransfer {
            generated.features.insert(CaseFeature::ControlledFlow);
        }
        return Ok(generated);
    }
    Err(GeneratorError::ConstraintAttemptsExhausted {
        target: "PPU sequence descriptor",
        attempts: STRUCTURED_ENCODING_ATTEMPTS,
    })
}

#[cfg(test)]
fn structured_word(
    descriptors: &[PpuGenerationDescriptor],
    rng: &mut Rng,
) -> Result<u32, GeneratorError> {
    structured_generated_word(descriptors, rng, None).map(|generated| generated.raw)
}

pub(super) fn structured_generated_word(
    descriptors: &[PpuGenerationDescriptor],
    rng: &mut Rng,
    forced_alias: Option<u32>,
) -> Result<GeneratedWord, GeneratorError> {
    if descriptors.is_empty() {
        return Err(GeneratorError::EmptyDescriptorRegistry { target: "PPU" });
    }
    let index = rng.below(descriptors.len() as u64)? as usize;
    let Some(descriptor) = descriptors.get(index) else {
        return Err(GeneratorError::EmptyDescriptorRegistry { target: "PPU" });
    };
    structured_generated_word_for_descriptor(descriptor, rng, forced_alias)
}

fn structured_generated_word_for_descriptor(
    descriptor: &PpuGenerationDescriptor,
    rng: &mut Rng,
    forced_alias: Option<u32>,
) -> Result<GeneratedWord, GeneratorError> {
    // [Yang2011 p:3 s:2.3] The generator drops a candidate the safety filter rejects and draws again until one passes.
    for _ in 0..STRUCTURED_ENCODING_ATTEMPTS {
        let parameters = generated_ppu_parameters(descriptor, rng, forced_alias)?;
        match descriptor.encode(parameters.stream.values()) {
            Ok(raw) => {
                return Ok(GeneratedWord {
                    raw,
                    features: parameters.features,
                })
            }
            Err(PpuGenerationError::InvalidOperands) => {}
            Err(error) => return Err(error.into()),
        }
    }
    Err(GeneratorError::ConstraintAttemptsExhausted {
        target: "PPU",
        attempts: STRUCTURED_ENCODING_ATTEMPTS,
    })
}

// [Padhye2019 p:332 s:3.1] Each random draw becomes one typed operand value, so every draw sequence encodes a structured word.
fn generated_ppu_parameters(
    descriptor: &PpuGenerationDescriptor,
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
        .filter(|field| field.class == PpuOperandClass::Register)
        .count();
    let mut features = BTreeSet::new();
    // [PPC-Book1 p:104 s:4.6.2] Equal field values can name separate FPR and GPR banks.
    if alias.is_some() && register_fields > 1 && descriptor.sequence_dependency.is_some() {
        features.insert(CaseFeature::OperandAlias);
    }
    let mut values = Vec::with_capacity(descriptor.operands.len());
    for field in &descriptor.operands {
        let value = if field.class == PpuOperandClass::Register && alias.is_some() {
            alias.unwrap_or(0) & field.maximum()
        } else if rng.chance(1, 4)? {
            features.insert(CaseFeature::OperandBoundary);
            let boundaries = field.boundary_values();
            let boundary_index = rng.below(boundaries.len() as u64)? as usize;
            boundaries.get(boundary_index).copied().ok_or(
                GeneratorError::EmptyDescriptorRegistry {
                    target: "PPU operand boundaries",
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

pub(super) fn random_state(rng: &mut Rng) -> Result<PpuState, GeneratorError> {
    let mut state = PpuState::new();
    let mut gpr = [0u64; 32];
    let mut fpr = [0u64; 32];
    let mut vr = [0u128; 32];
    for value in &mut gpr {
        *value = if rng.chance(1, 3)? {
            DATA_BASE + rng.below(DATA_LEN as u64)?
        } else {
            rng.mixed_u64()?
        };
    }
    for value in &mut fpr {
        *value = rng.fp_bits()?;
    }
    for value in &mut vr {
        *value = (u128::from(rng.next_u64()) << 64) | u128::from(rng.next_u64());
    }
    state.set_gpr_all(gpr);
    state.set_fpr_all(fpr);
    state.set_vr_all(vr);
    state.pc = rng.next_u64() & !3;
    state.set_cr(rng.next_u32());
    state.set_lr(rng.next_u64());
    state.set_ctr(rng.next_u64());
    state.set_xer(rng.next_u64());
    state.vrsave = rng.next_u32();
    // The executor treats a seeded VRSAVE value as initialized, as after `mtvrsave`.
    state.vrsave_written = true;
    state.tb = rng.next_u64();
    state.set_reservation(if rng.chance(1, 2)? {
        Some(ReservedLine::containing(
            DATA_BASE + rng.below(DATA_LEN as u64)?,
        ))
    } else {
        None
    });
    Ok(state)
}

fn random_state_for_instruction(
    instruction: &PpuInstruction,
    rng: &mut Rng,
) -> Result<PpuState, GeneratorError> {
    let mut state = random_state(rng)?;
    // [Wang2024 p:340:10 s:3.2] The generator filters candidate inputs to values that satisfy the instruction's precondition before it uses one.
    if let PpuInstruction::Lswx { rt, ra, rb } = *instruction {
        // [PPC-Book1 p:48 s:3.3] The XER byte count determines the wrapping
        // destination-register range, which must exclude both address registers.
        let valid_counts = (1..=127u8)
            .filter(|count| lswx_registers_are_valid(rt, ra, rb, *count))
            .collect::<Vec<_>>();
        if valid_counts.is_empty() {
            return Err(GeneratorError::ConstraintAttemptsExhausted {
                target: "PPU lswx state",
                attempts: 127,
            });
        }
        let index = rng.below(valid_counts.len() as u64)? as usize;
        let count = valid_counts.get(index).copied().ok_or(
            GeneratorError::ConstraintAttemptsExhausted {
                target: "PPU lswx state selection",
                attempts: valid_counts.len(),
            },
        )?;
        state.set_xer((state.xer() & !0x7f) | u64::from(count));
    }
    Ok(state)
}

pub(super) fn state_aware_state_for_instruction(
    instruction: &PpuInstruction,
    rng: &mut Rng,
) -> Result<(PpuState, BTreeSet<CaseFeature>), GeneratorError> {
    let mut state = random_state_for_instruction(instruction, rng)?;
    bias_ppu_address_state(&mut state);
    Ok((
        state,
        BTreeSet::from([CaseFeature::MappedMemory, CaseFeature::Reservation]),
    ))
}

fn random_state_for_sequence(
    strategy: GenerationStrategy,
    rng: &mut Rng,
) -> Result<PpuState, GeneratorError> {
    let mut state = random_state(rng)?;
    match strategy {
        GenerationStrategy::Structured => state.set_xer((state.xer() & !0x7f) | 1),
        GenerationStrategy::RawWords => {}
    }
    Ok(state)
}

pub(super) fn state_aware_state_for_sequence(
    strategy: GenerationStrategy,
    rng: &mut Rng,
) -> Result<(PpuState, BTreeSet<CaseFeature>), GeneratorError> {
    let mut state = random_state_for_sequence(strategy, rng)?;
    let mut features = BTreeSet::new();
    if strategy == GenerationStrategy::Structured {
        bias_ppu_address_state(&mut state);
        features.extend([CaseFeature::MappedMemory, CaseFeature::Reservation]);
    }
    Ok((state, features))
}

fn bias_ppu_address_state(state: &mut PpuState) {
    for register in 0..32 {
        let value = match register % 4 {
            0 => 0,
            1 => DATA_BASE,
            2 => DATA_BASE + 64,
            _ => DATA_BASE - 64,
        };
        state.set_gpr(register, value);
    }
    state.set_reservation(Some(ReservedLine::containing(DATA_BASE)));
}

fn lswx_registers_are_valid(rt: u8, ra: u8, rb: u8, byte_count: u8) -> bool {
    let register_count = byte_count.div_ceil(4);
    !(0..register_count).any(|index| {
        let destination = rt.wrapping_add(index) & 31;
        destination == ra || destination == rb
    })
}

#[cfg(test)]
#[path = "tests/generate_tests.rs"]
mod tests;
