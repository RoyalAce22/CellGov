//! SPU fuzz engines built on interpreter-owned descriptors.

use cellgov_effects::Effect;
use cellgov_event::UnitId;
use cellgov_spu::exec::{execute, SpuStepOutcome};
use cellgov_spu::fuzz::{
    generation_descriptors, SpuGenerationDescriptor, SpuGenerationError, SpuMetamorphicRelation,
    SpuOperandClass, SpuOutcomeClass,
};
use cellgov_spu::state::{SpuObservableSnapshot, SpuState, SPU_LS_SIZE};
use cellgov_sync::ReservedLine;

use crate::boundary::{call_harness, call_target};
use crate::error::{FuzzError, GeneratorError, InvariantError};
use crate::report::{
    CheckIdentity, DivergenceClass, Finding, FindingKind, FuzzReport, FuzzRun, FuzzTarget,
    InstructionIdentity, OutcomeIdentity, ReductionOutcome, SemanticFingerprint,
};
use crate::rng::{Rng, WordGenerationFailure};
use crate::{
    FuzzConfig, GenerationStrategy, ParameterStream, ReplayCoordinates, TargetPanicPayload,
};

const UNIT: UnitId = UnitId::new(0);
const STRUCTURED_ENCODING_ATTEMPTS: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq)]
struct ObservedStep {
    outcome: SpuStepOutcome,
    state: SpuObservableSnapshot,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ObservedSequence {
    state: SpuObservableSnapshot,
    terminal_outcome: Option<SpuStepOutcome>,
    decode_refusal: Option<(u32, u32)>,
    deterministic: bool,
}

/// Checks each decoded SPU instruction against its descriptor.
pub fn run_instructions(config: FuzzConfig) -> FuzzRun {
    guarded_run(FuzzTarget::SpuInstruction, config, |report| {
        run_instructions_inner(config, report)
    })
}

fn run_instructions_inner(config: FuzzConfig, report: &mut FuzzReport) -> Result<(), FuzzError> {
    config.validate(None)?;
    let iterations = config.case_indices()?;
    let descriptors = match (config.strategy, iterations.clone().next()) {
        (GenerationStrategy::Structured, Some(first)) => {
            match call_target(generation_descriptors) {
                Ok(descriptors) => descriptors,
                Err(payload) => {
                    report.considered()?;
                    record_target_panic(
                        report,
                        CheckIdentity::SpuDecoder,
                        None,
                        Vec::new(),
                        first,
                        payload,
                    )?;
                    return Ok(());
                }
            }
        }
        (GenerationStrategy::Structured | GenerationStrategy::RawWords, _) => Vec::new(),
    };
    for iteration in iterations {
        report.considered()?;
        let mut rng = Rng::for_case(config.campaign_version, config.seed, iteration);
        let raw = match config.strategy {
            GenerationStrategy::Structured => {
                match call_target(|| structured_word(&descriptors, &mut rng)) {
                    Ok(Ok(raw)) => raw,
                    Ok(Err(error)) => return Err(error.into()),
                    Err(payload) => {
                        record_target_panic(
                            report,
                            CheckIdentity::SpuDecoder,
                            None,
                            Vec::new(),
                            iteration,
                            payload,
                        )?;
                        continue;
                    }
                }
            }
            GenerationStrategy::RawWords => rng.next_u32(),
        };
        let decoded = match call_target(|| cellgov_spu::decode::decode(raw)) {
            Ok(decoded) => decoded,
            Err(payload) => {
                record_target_panic(
                    report,
                    CheckIdentity::SpuDecoder,
                    None,
                    vec![raw],
                    iteration,
                    payload,
                )?;
                continue;
            }
        };
        let Ok(instruction) = decoded else { continue };
        let descriptor = instruction.fuzz_descriptor();
        let identity = InstructionIdentity::Spu(descriptor.kind);
        report.reached(identity)?;
        let initial = random_state(&mut rng)?;
        let first = match call_target(|| run_once(&instruction, &initial)) {
            Ok(first) => first,
            Err(payload) => {
                record_target_panic(
                    report,
                    CheckIdentity::SpuExecutor,
                    Some(identity),
                    vec![raw],
                    iteration,
                    payload,
                )?;
                continue;
            }
        };
        if requests_replay(descriptor.relations) {
            let second = match call_target(|| run_once(&instruction, &initial)) {
                Ok(second) => second,
                Err(payload) => {
                    record_target_panic(
                        report,
                        CheckIdentity::SpuExecutor,
                        Some(identity),
                        vec![raw],
                        iteration,
                        payload,
                    )?;
                    continue;
                }
            };
            if first != second {
                record(
                    report,
                    FindingKind::Nondeterministic,
                    SemanticFingerprint {
                        target: FuzzTarget::SpuInstruction,
                        instruction_kind: Some(identity),
                        check: CheckIdentity::DeterministicReplay,
                        divergence: DivergenceClass::ArchitecturalState,
                        outcome: None,
                        effect: None,
                    },
                    vec![raw],
                    iteration,
                )?;
            }
        }
        let outcome = SpuOutcomeClass::from_outcome(&first.outcome);
        if !descriptor.outcomes.contains(&outcome) {
            record(
                report,
                FindingKind::IllegalOutcome,
                SemanticFingerprint {
                    target: FuzzTarget::SpuInstruction,
                    instruction_kind: Some(identity),
                    check: CheckIdentity::LegalOutcome,
                    divergence: DivergenceClass::Outcome,
                    outcome: Some(outcome_identity(outcome)),
                    effect: None,
                },
                vec![raw],
                iteration,
            )?;
        }
        if let Some(effect) = outcome_effects(&first.outcome)
            .iter()
            .map(Effect::kind)
            .find(|effect| !descriptor.effects.contains(effect))
        {
            record(
                report,
                FindingKind::IllegalEffect,
                SemanticFingerprint {
                    target: FuzzTarget::SpuInstruction,
                    instruction_kind: Some(identity),
                    check: CheckIdentity::LegalEffect,
                    divergence: DivergenceClass::Effect,
                    outcome: None,
                    effect: Some(effect),
                },
                vec![raw],
                iteration,
            )?;
        }
    }
    Ok(())
}

/// Replays SPU instruction sequences to find nondeterministic outcomes.
pub fn run_sequences(config: FuzzConfig) -> FuzzRun {
    guarded_run(FuzzTarget::SpuSequence, config, |report| {
        run_sequences_inner(config, report)
    })
}

fn run_sequences_inner(config: FuzzConfig, report: &mut FuzzReport) -> Result<(), FuzzError> {
    config.validate(Some((SPU_LS_SIZE / 4).min(crate::MAX_SEQUENCE_WORDS)))?;
    let iterations = config.case_indices()?;
    let descriptors = match (config.strategy, iterations.clone().next()) {
        (GenerationStrategy::Structured, Some(first)) => {
            match call_target(generation_descriptors) {
                Ok(descriptors) => descriptors,
                Err(payload) => {
                    report.considered()?;
                    record_target_panic(
                        report,
                        CheckIdentity::SpuDecoder,
                        None,
                        Vec::new(),
                        first,
                        payload,
                    )?;
                    return Ok(());
                }
            }
        }
        (GenerationStrategy::Structured | GenerationStrategy::RawWords, _) => Vec::new(),
    };
    for iteration in iterations {
        report.considered()?;
        let mut rng = Rng::for_case(config.campaign_version, config.seed, iteration);
        let words = match config.strategy {
            GenerationStrategy::Structured => {
                match call_target(|| {
                    structured_words(&descriptors, &mut rng, config.sequence_words as usize)
                }) {
                    Ok(Ok(words)) => Ok(words),
                    Ok(Err(error)) => Err(WordGenerationFailure::Exhausted(error)),
                    Err(payload) => Err(WordGenerationFailure::DecoderPanic(0, payload)),
                }
            }
            GenerationStrategy::RawWords => {
                rng.decoder_accepted_words(config.sequence_words as usize, |raw| {
                    call_target(|| cellgov_spu::decode::decode(raw)).map(|decoded| decoded.is_ok())
                })
            }
        };
        let words = match words {
            Ok(words) => words,
            Err(WordGenerationFailure::DecoderPanic(raw, payload)) => {
                record_target_panic(
                    report,
                    CheckIdentity::SpuDecoder,
                    None,
                    vec![raw],
                    iteration,
                    payload,
                )?;
                continue;
            }
            Err(WordGenerationFailure::Exhausted(error)) => return Err(error.into()),
        };
        if words.is_empty() {
            return Err(InvariantError::EmptyGeneratedSequence.into());
        }
        let mut initial = random_state(&mut rng)?;
        initial.pc = 0;
        for (index, word) in words.iter().enumerate() {
            let start = index * 4;
            if start + 4 > initial.ls.len() {
                break;
            }
            initial.ls[start..start + 4].copy_from_slice(&word.to_be_bytes());
        }
        let first = match call_target(|| {
            run_generated_sequence(&initial, config.sequence_words as usize, config.strategy)
        }) {
            Ok(first) => first,
            Err(payload) => {
                record_target_panic(
                    report,
                    CheckIdentity::SpuExecutor,
                    None,
                    words.clone(),
                    iteration,
                    payload,
                )?;
                continue;
            }
        };
        report.reached_many(first.1, first.2.iter().copied())?;
        if first.0.deterministic {
            let second = match call_target(|| {
                run_generated_sequence(&initial, config.sequence_words as usize, config.strategy)
            }) {
                Ok(second) => second,
                Err(payload) => {
                    record_target_panic(
                        report,
                        CheckIdentity::SpuExecutor,
                        None,
                        words.clone(),
                        iteration,
                        payload,
                    )?;
                    continue;
                }
            };
            if first.0 != second.0 {
                record(
                    report,
                    FindingKind::Nondeterministic,
                    SemanticFingerprint {
                        target: FuzzTarget::SpuSequence,
                        instruction_kind: None,
                        check: CheckIdentity::DeterministicReplay,
                        divergence: DivergenceClass::ArchitecturalState,
                        outcome: None,
                        effect: None,
                    },
                    words.clone(),
                    iteration,
                )?;
            }
        }
        if first.0.state.pc as usize >= SPU_LS_SIZE || first.0.state.pc & 3 != 0 {
            record(
                report,
                FindingKind::InvalidProgramCounter,
                SemanticFingerprint {
                    target: FuzzTarget::SpuSequence,
                    instruction_kind: None,
                    check: CheckIdentity::ProgramCounter,
                    divergence: DivergenceClass::ControlFlow,
                    outcome: None,
                    effect: None,
                },
                words.clone(),
                iteration,
            )?;
        }
    }
    Ok(())
}

fn run_once(
    instruction: &cellgov_spu::instruction::SpuInstruction,
    initial: &SpuState,
) -> ObservedStep {
    let mut state = initial.clone();
    let outcome = execute(instruction, &mut state, UNIT);
    ObservedStep {
        outcome,
        state: SpuObservableSnapshot::capture(&state),
    }
}

fn run_sequence(
    initial: &SpuState,
    budget: usize,
) -> (ObservedSequence, u64, Vec<InstructionIdentity>) {
    run_sequence_with_limit(initial, budget, None)
}

fn run_generated_sequence(
    initial: &SpuState,
    budget: usize,
    strategy: GenerationStrategy,
) -> (ObservedSequence, u64, Vec<InstructionIdentity>) {
    match strategy {
        GenerationStrategy::Structured => run_sequence_with_limit(initial, budget, Some(budget)),
        GenerationStrategy::RawWords => run_sequence(initial, budget),
    }
}

fn run_sequence_with_limit(
    initial: &SpuState,
    budget: usize,
    program_words: Option<usize>,
) -> (ObservedSequence, u64, Vec<InstructionIdentity>) {
    let mut state = initial.clone();
    let mut decoded = 0u64;
    let mut kinds = Vec::new();
    let mut terminal_outcome = None;
    let mut decode_refusal = None;
    let mut deterministic = true;
    for _ in 0..budget {
        let Some(slot) = usize::try_from(state.pc / 4).ok() else {
            break;
        };
        // Stop structured execution at the program bound because the finding records no other
        // local-store words.
        if program_words.is_some_and(|word_count| slot >= word_count) {
            break;
        }
        let Some(raw) = state.fetch() else {
            break;
        };
        let Ok(instruction) = cellgov_spu::decode::decode(raw) else {
            decode_refusal = Some((state.pc, raw));
            break;
        };
        let descriptor = instruction.fuzz_descriptor();
        deterministic &= requests_replay(descriptor.relations);
        decoded += 1;
        kinds.push(InstructionIdentity::Spu(descriptor.kind));
        let outcome = execute(&instruction, &mut state, UNIT);
        terminal_outcome = Some(outcome.clone());
        match outcome {
            SpuStepOutcome::Continue => state.pc = state.pc.wrapping_add(4),
            SpuStepOutcome::Branch => {}
            SpuStepOutcome::Yield { .. } | SpuStepOutcome::MemoryRead { .. } => break,
            SpuStepOutcome::Fault(_) => {
                // The runtime fault-discard rule hides state from a faulting batch.
                state = initial.clone();
                break;
            }
        }
    }
    (
        ObservedSequence {
            state: SpuObservableSnapshot::capture(&state),
            // Sequence replay retains the terminal outcome because the descriptor observes its effects.
            terminal_outcome,
            decode_refusal,
            deterministic,
        },
        decoded,
        kinds,
    )
}

fn structured_words(
    descriptors: &[SpuGenerationDescriptor],
    rng: &mut Rng,
    count: usize,
) -> Result<Vec<u32>, GeneratorError> {
    (0..count)
        .map(|_| structured_word(descriptors, rng))
        .collect()
}

fn structured_word(
    descriptors: &[SpuGenerationDescriptor],
    rng: &mut Rng,
) -> Result<u32, GeneratorError> {
    if descriptors.is_empty() {
        return Err(GeneratorError::EmptyDescriptorRegistry { target: "SPU" });
    }
    let index = rng.below(descriptors.len() as u64)? as usize;
    let Some(descriptor) = descriptors.get(index) else {
        return Err(GeneratorError::EmptyDescriptorRegistry { target: "SPU" });
    };
    // [Yang2011 p:1 s:Abstract] Only valid typed operand combinations reach comparison.
    for _ in 0..STRUCTURED_ENCODING_ATTEMPTS {
        let parameters = spu_parameters(descriptor, rng)?;
        match descriptor.encode(parameters.values()) {
            Ok(raw) => return Ok(raw),
            Err(SpuGenerationError::InvalidOperands) => {}
            Err(error) => return Err(error.into()),
        }
    }
    Err(GeneratorError::ConstraintAttemptsExhausted {
        target: "SPU",
        attempts: STRUCTURED_ENCODING_ATTEMPTS,
    })
}

fn spu_parameters(
    descriptor: &SpuGenerationDescriptor,
    rng: &mut Rng,
) -> Result<ParameterStream, GeneratorError> {
    let alias = rng.chance(1, 8)?.then(|| rng.next_u32());
    let mut values = Vec::with_capacity(descriptor.operands.len());
    for field in &descriptor.operands {
        let value = if field.class == SpuOperandClass::Register && alias.is_some() {
            alias.unwrap_or(0) & field.maximum()
        } else if rng.chance(1, 4)? {
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
    Ok(ParameterStream::new(values))
}

fn random_state(rng: &mut Rng) -> Result<SpuState, FuzzError> {
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

fn requests_replay(relations: &[SpuMetamorphicRelation]) -> bool {
    relations.contains(&SpuMetamorphicRelation::Deterministic)
}

fn outcome_identity(outcome: SpuOutcomeClass) -> OutcomeIdentity {
    match outcome {
        SpuOutcomeClass::Continue => OutcomeIdentity::SpuContinue,
        SpuOutcomeClass::Branch => OutcomeIdentity::SpuBranch,
        SpuOutcomeClass::Yield => OutcomeIdentity::SpuYield,
        SpuOutcomeClass::MemoryRead => OutcomeIdentity::SpuMemoryRead,
        SpuOutcomeClass::Fault => OutcomeIdentity::SpuFault,
    }
}

fn outcome_effects(outcome: &SpuStepOutcome) -> &[Effect] {
    match outcome {
        SpuStepOutcome::Yield { effects, .. } => effects,
        SpuStepOutcome::Continue
        | SpuStepOutcome::Branch
        | SpuStepOutcome::MemoryRead { .. }
        | SpuStepOutcome::Fault(_) => &[],
    }
}

fn record(
    report: &mut FuzzReport,
    kind: FindingKind,
    fingerprint: SemanticFingerprint,
    original_words: Vec<u32>,
    iteration: u64,
) -> Result<(), InvariantError> {
    report.finding(Finding {
        fingerprint,
        kind,
        replay: ReplayCoordinates::new(
            report.target,
            report.strategy,
            report.seed,
            iteration,
            report.sequence_words,
        ),
        original_words,
        reduction: ReductionOutcome::NotAttempted,
        panic_payload: None,
    })
}

fn record_target_panic(
    report: &mut FuzzReport,
    check: CheckIdentity,
    instruction_kind: Option<InstructionIdentity>,
    original_words: Vec<u32>,
    iteration: u64,
    payload: TargetPanicPayload,
) -> Result<(), InvariantError> {
    report.finding(Finding {
        fingerprint: SemanticFingerprint {
            target: report.target,
            instruction_kind,
            check,
            divergence: DivergenceClass::TargetPanic,
            outcome: None,
            effect: None,
        },
        kind: FindingKind::TargetPanic,
        replay: ReplayCoordinates::new(
            report.target,
            report.strategy,
            report.seed,
            iteration,
            report.sequence_words,
        ),
        original_words,
        reduction: ReductionOutcome::NotAttempted,
        panic_payload: Some(payload),
    })
}

fn guarded_run(
    target: FuzzTarget,
    config: FuzzConfig,
    run: impl FnOnce(&mut FuzzReport) -> Result<(), FuzzError>,
) -> FuzzRun {
    let mut report = FuzzReport::new(
        target,
        config.seed,
        config.strategy,
        config.max_findings as usize,
        config.sequence_words,
    );
    match call_harness(|| run(&mut report)) {
        Ok(Ok(())) if config.schedule.is_cancelled() && report.is_clean() => {
            FuzzRun::cancelled(report)
        }
        Ok(Ok(())) => FuzzRun::completed(report),
        Ok(Err(error)) => FuzzRun::failed(report, error),
        Err(_) => FuzzRun::failed(
            report,
            InvariantError::UnexpectedPanic {
                stage: "SPU campaign",
            },
        ),
    }
}

#[cfg(test)]
#[path = "tests/spu_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/structured_sequence_tests.rs"]
mod structured_sequence_tests;
