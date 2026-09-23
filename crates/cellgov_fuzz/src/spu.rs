//! SPU fuzz engines built on interpreter-owned descriptors.

use std::collections::BTreeSet;

use cellgov_effects::Effect;
use cellgov_event::UnitId;
use cellgov_spu::exec::{execute, SpuStepOutcome};
use cellgov_spu::fuzz::{
    encoding_execution_is_supported, encoding_has_undefined_operands, generation_descriptors,
    SpuFuzzDescriptor, SpuGenerationDescriptor, SpuGenerationError, SpuMetamorphicRelation,
    SpuOperandClass, SpuOutcomeClass, SpuRelationRefusal, SpuSequenceFlow, SpuSequenceInteraction,
    SpuStateInput,
};
use cellgov_spu::observation::{SpuAllowedFootprint, SpuObservation, SpuObservationComponent};
use cellgov_spu::state::{SpuObservableSnapshot, SpuState, SPU_LS_SIZE};
use cellgov_sync::{ReservedLine, RESERVATION_LINE_BYTES};

use crate::boundary::{call_harness, call_target};
use crate::case::{CaseAssessment, CaseEligibility, CaseFeature, EligibilityReason};
use crate::error::{FuzzError, GeneratorError, InvariantError};
use crate::report::{
    CheckIdentity, DivergenceClass, Finding, FindingKind, FuzzReport, FuzzRun, FuzzTarget,
    InstructionIdentity, OutcomeIdentity, ReductionOutcome, SemanticFingerprint,
};
use crate::retention::{CrossReferenceAsymmetry, SemanticObservation, StateTransitionClass};
use crate::rng::{Rng, WordGenerationFailure};
use crate::{
    FuzzConfig, GenerationStrategy, ParameterStream, ReplayCoordinates, TargetPanicPayload,
};

const UNIT: UnitId = UnitId::new(0);
const STRUCTURED_ENCODING_ATTEMPTS: usize = 64;
const STRUCTURED_LS_DATA_BASE: u32 = 0x1_0000;

#[derive(Debug, Clone, PartialEq, Eq)]
struct GeneratedWord {
    raw: u32,
    features: BTreeSet<CaseFeature>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct GeneratedSequence {
    words: Vec<u32>,
    features: BTreeSet<CaseFeature>,
    interaction: Option<(SpuSequenceInteraction, u32)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct GeneratedParameters {
    stream: ParameterStream,
    features: BTreeSet<CaseFeature>,
}

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
    has_undefined_operands: bool,
    has_unmodeled_execution: bool,
    footprint_violations: BTreeSet<SpuObservationComponent>,
}

#[derive(Clone, Copy)]
enum SpuTerminalObservation<'a> {
    Execution(Option<&'a SpuStepOutcome>),
    DecodeRefusal(Option<&'a SpuStepOutcome>),
}

impl<'a> SpuTerminalObservation<'a> {
    fn from_sequence(observed: &'a ObservedSequence) -> Self {
        if observed.decode_refusal.is_some() {
            Self::DecodeRefusal(observed.terminal_outcome.as_ref())
        } else {
            Self::Execution(observed.terminal_outcome.as_ref())
        }
    }

    fn outcome(self) -> Option<&'a SpuStepOutcome> {
        match self {
            Self::Execution(outcome) | Self::DecodeRefusal(outcome) => outcome,
        }
    }
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
        let generated = match config.strategy {
            GenerationStrategy::Structured => {
                match call_target(|| structured_generated_word(&descriptors, &mut rng, None)) {
                    Ok(Ok(generated)) => generated,
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
            GenerationStrategy::RawWords => GeneratedWord {
                raw: rng.next_u32(),
                features: BTreeSet::new(),
            },
        };
        let raw = generated.raw;
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
        let (initial, state_features) = match config.strategy {
            GenerationStrategy::Structured => state_aware_state(&mut rng, descriptor.state_input)?,
            GenerationStrategy::RawWords => (random_state(&mut rng)?, BTreeSet::new()),
        };
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
        let mut features = generated.features;
        features.extend(state_features);
        let assessment = assess_instruction_case(
            config.strategy,
            encoding_has_undefined_operands(raw),
            encoding_execution_is_supported(raw),
            descriptor,
            &first.outcome,
            features,
        );
        report.assessed(&assessment)?;
        report.executed(1)?;
        report.observed_effects(outcome_effects(&first.outcome).iter().map(Effect::kind))?;
        if assessment.eligibility != CaseEligibility::Eligible {
            report.observe_case(
                iteration,
                spu_observation(
                    [identity],
                    &assessment,
                    SpuTerminalObservation::Execution(Some(&first.outcome)),
                    &first.state,
                    SpuObservableSnapshot::capture(&initial),
                    1,
                    CrossReferenceAsymmetry::None,
                ),
            )?;
            continue;
        }
        let mut asymmetry = CrossReferenceAsymmetry::None;
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
                    report.observe_case(
                        iteration,
                        spu_observation(
                            [identity],
                            &assessment,
                            SpuTerminalObservation::Execution(Some(&first.outcome)),
                            &first.state,
                            SpuObservableSnapshot::capture(&initial),
                            1,
                            CrossReferenceAsymmetry::TargetPanic,
                        ),
                    )?;
                    continue;
                }
            };
            let replay_asymmetry = spu_step_replay_asymmetry(&first, &second);
            if replay_asymmetry != CrossReferenceAsymmetry::None {
                asymmetry = asymmetry.max(replay_asymmetry);
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
            asymmetry = asymmetry.max(spu_outcome_class_asymmetry(outcome));
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
            asymmetry = asymmetry.max(CrossReferenceAsymmetry::Effect);
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
        let complete = SpuObservation::from_parts(first.state.clone(), first.outcome.clone());
        for component in
            SpuAllowedFootprint::for_instruction(&instruction).violations(&initial, &complete)
        {
            let divergence = match component {
                SpuObservationComponent::ProgramCounter => DivergenceClass::ControlFlow,
                SpuObservationComponent::Effects => DivergenceClass::Effect,
                SpuObservationComponent::Registers
                | SpuObservationComponent::LocalStore
                | SpuObservationComponent::Channels
                | SpuObservationComponent::Reservation
                | SpuObservationComponent::Outcome
                | SpuObservationComponent::FaultDiscard => DivergenceClass::ArchitecturalState,
            };
            asymmetry = asymmetry.max(CrossReferenceAsymmetry::State);
            record(
                report,
                FindingKind::IllegalFootprint,
                SemanticFingerprint {
                    target: FuzzTarget::SpuInstruction,
                    instruction_kind: Some(identity),
                    check: CheckIdentity::AllowedFootprint,
                    divergence,
                    outcome: Some(outcome_identity(SpuOutcomeClass::from_outcome(
                        &first.outcome,
                    ))),
                    effect: None,
                },
                vec![raw],
                iteration,
            )?;
        }
        asymmetry = asymmetry.max(run_metamorphic_checks(
            report,
            &instruction,
            descriptor,
            &initial,
            &first,
            raw,
            identity,
            iteration,
        )?);
        report.observe_case(
            iteration,
            spu_observation(
                [identity],
                &assessment,
                SpuTerminalObservation::Execution(Some(&first.outcome)),
                &first.state,
                SpuObservableSnapshot::capture(&initial),
                1,
                asymmetry,
            ),
        )?;
    }
    Ok(())
}

fn spu_relation_check(relation: SpuMetamorphicRelation) -> CheckIdentity {
    match relation {
        SpuMetamorphicRelation::Deterministic => CheckIdentity::DeterministicReplay,
        SpuMetamorphicRelation::NopFalseTarget => CheckIdentity::SpuNopFalseTarget,
        SpuMetamorphicRelation::RotateByteCountHighBit => CheckIdentity::SpuRotateByteCountHighBit,
    }
}

#[allow(clippy::too_many_arguments)]
fn run_metamorphic_checks(
    report: &mut FuzzReport,
    instruction: &cellgov_spu::instruction::SpuInstruction,
    descriptor: SpuFuzzDescriptor,
    initial: &SpuState,
    first: &ObservedStep,
    raw: u32,
    identity: InstructionIdentity,
    iteration: u64,
) -> Result<CrossReferenceAsymmetry, FuzzError> {
    let mut strongest = CrossReferenceAsymmetry::None;
    for &relation in descriptor
        .relations
        .iter()
        .filter(|relation| **relation != SpuMetamorphicRelation::Deterministic)
    {
        let check = spu_relation_check(relation);
        let case = match call_target(|| instruction.metamorphic_case(raw, relation)) {
            Ok(Ok(case)) => case,
            Ok(Err(
                SpuRelationRefusal::NoPartner { .. } | SpuRelationRefusal::Ineligible { .. },
            )) => {
                report.metamorphic_skipped(check)?;
                continue;
            }
            Ok(Err(
                SpuRelationRefusal::Undeclared { .. } | SpuRelationRefusal::InvalidPartner { .. },
            )) => {
                strongest = strongest.max(CrossReferenceAsymmetry::Outcome);
                record(
                    report,
                    FindingKind::MetamorphicViolation,
                    SemanticFingerprint {
                        target: FuzzTarget::SpuInstruction,
                        instruction_kind: Some(identity),
                        check,
                        divergence: DivergenceClass::ReferenceDisagreement,
                        outcome: None,
                        effect: None,
                    },
                    vec![raw],
                    iteration,
                )?;
                continue;
            }
            Err(payload) => {
                record_target_panic(report, check, Some(identity), vec![raw], iteration, payload)?;
                strongest = strongest.max(CrossReferenceAsymmetry::TargetPanic);
                continue;
            }
        };
        let partner = match call_target(|| {
            let decoded = cellgov_spu::decode::decode(case.partner_word);
            decoded.map(|instruction| run_once(&instruction, initial))
        }) {
            Ok(Ok(partner)) => partner,
            Ok(Err(_)) => {
                strongest = strongest.max(CrossReferenceAsymmetry::Outcome);
                record(
                    report,
                    FindingKind::MetamorphicViolation,
                    SemanticFingerprint {
                        target: FuzzTarget::SpuInstruction,
                        instruction_kind: Some(identity),
                        check,
                        divergence: DivergenceClass::Outcome,
                        outcome: None,
                        effect: None,
                    },
                    vec![raw, case.partner_word],
                    iteration,
                )?;
                continue;
            }
            Err(payload) => {
                strongest = strongest.max(CrossReferenceAsymmetry::TargetPanic);
                record_target_panic(
                    report,
                    check,
                    Some(identity),
                    vec![raw, case.partner_word],
                    iteration,
                    payload,
                )?;
                continue;
            }
        };
        report.metamorphic_executed(check)?;
        let differences = SpuObservation::from_parts(first.state.clone(), first.outcome.clone())
            .compare(&SpuObservation::from_parts(partner.state, partner.outcome))
            .differences;
        if differences.is_empty() {
            continue;
        }
        let (divergence, asymmetry) =
            if differences.contains(&SpuObservationComponent::FaultDiscard) {
                (DivergenceClass::Outcome, CrossReferenceAsymmetry::Fault)
            } else if differences.contains(&SpuObservationComponent::Effects) {
                (DivergenceClass::Effect, CrossReferenceAsymmetry::Effect)
            } else if differences.contains(&SpuObservationComponent::Outcome) {
                (DivergenceClass::Outcome, CrossReferenceAsymmetry::Outcome)
            } else if differences.contains(&SpuObservationComponent::ProgramCounter) {
                (DivergenceClass::ControlFlow, CrossReferenceAsymmetry::State)
            } else {
                (
                    DivergenceClass::ArchitecturalState,
                    CrossReferenceAsymmetry::State,
                )
            };
        strongest = strongest.max(asymmetry);
        record(
            report,
            FindingKind::MetamorphicViolation,
            SemanticFingerprint {
                target: FuzzTarget::SpuInstruction,
                instruction_kind: Some(identity),
                check,
                divergence,
                outcome: None,
                effect: None,
            },
            vec![raw, case.partner_word],
            iteration,
        )?;
    }
    Ok(strongest)
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
        let generated = match config.strategy {
            GenerationStrategy::Structured => {
                match call_target(|| {
                    structured_sequence(&descriptors, &mut rng, config.sequence_words as usize)
                }) {
                    Ok(Ok(generated)) => Ok(generated),
                    Ok(Err(error)) => Err(WordGenerationFailure::Exhausted(error)),
                    Err(payload) => Err(WordGenerationFailure::DecoderPanic(0, payload)),
                }
            }
            GenerationStrategy::RawWords => rng
                .decoder_accepted_words(config.sequence_words as usize, |raw| {
                    call_target(|| cellgov_spu::decode::decode(raw)).map(|decoded| decoded.is_ok())
                })
                .map(|words| GeneratedSequence {
                    words,
                    features: BTreeSet::new(),
                    interaction: None,
                }),
        };
        let generated = match generated {
            Ok(generated) => generated,
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
        let words = generated.words;
        if words.is_empty() {
            return Err(InvariantError::EmptyGeneratedSequence.into());
        }
        let (mut initial, state_features) = match config.strategy {
            GenerationStrategy::Structured => state_aware_state(&mut rng, None)?,
            GenerationStrategy::RawWords => (random_state(&mut rng)?, BTreeSet::new()),
        };
        if let Some((interaction, data_base)) = generated.interaction {
            interaction.prepare_state(&mut initial, data_base);
        }
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
        let mut features = generated.features;
        features.extend(state_features);
        let assessment = assess_sequence_case(
            config.strategy,
            first.1,
            first.0.has_undefined_operands,
            first.0.has_unmodeled_execution,
            features,
        );
        report.assessed(&assessment)?;
        report.executed(first.1)?;
        report.observed_effects(
            first
                .0
                .terminal_outcome
                .as_ref()
                .into_iter()
                .flat_map(outcome_effects)
                .map(Effect::kind),
        )?;
        if assessment.eligibility != CaseEligibility::Eligible {
            report.observe_case(
                iteration,
                spu_observation(
                    first.2.iter().copied(),
                    &assessment,
                    SpuTerminalObservation::from_sequence(&first.0),
                    &first.0.state,
                    SpuObservableSnapshot::capture(&initial),
                    first.1,
                    CrossReferenceAsymmetry::None,
                ),
            )?;
            continue;
        }
        let mut asymmetry = CrossReferenceAsymmetry::None;
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
                    report.observe_case(
                        iteration,
                        spu_observation(
                            first.2.iter().copied(),
                            &assessment,
                            SpuTerminalObservation::from_sequence(&first.0),
                            &first.0.state,
                            SpuObservableSnapshot::capture(&initial),
                            first.1,
                            CrossReferenceAsymmetry::TargetPanic,
                        ),
                    )?;
                    continue;
                }
            };
            let replay_asymmetry = spu_sequence_replay_asymmetry(&first, &second);
            if replay_asymmetry != CrossReferenceAsymmetry::None {
                asymmetry = asymmetry.max(replay_asymmetry);
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
            asymmetry = asymmetry.max(CrossReferenceAsymmetry::Fault);
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
        for component in &first.0.footprint_violations {
            let divergence = match component {
                SpuObservationComponent::ProgramCounter => DivergenceClass::ControlFlow,
                SpuObservationComponent::Effects => DivergenceClass::Effect,
                SpuObservationComponent::Outcome | SpuObservationComponent::FaultDiscard => {
                    DivergenceClass::Outcome
                }
                SpuObservationComponent::Registers
                | SpuObservationComponent::LocalStore
                | SpuObservationComponent::Channels
                | SpuObservationComponent::Reservation => DivergenceClass::ArchitecturalState,
            };
            asymmetry = asymmetry.max(CrossReferenceAsymmetry::State);
            record(
                report,
                FindingKind::IllegalFootprint,
                SemanticFingerprint {
                    target: FuzzTarget::SpuSequence,
                    instruction_kind: None,
                    check: CheckIdentity::AllowedFootprint,
                    divergence,
                    outcome: first
                        .0
                        .terminal_outcome
                        .as_ref()
                        .map(|outcome| outcome_identity(SpuOutcomeClass::from_outcome(outcome))),
                    effect: None,
                },
                words.clone(),
                iteration,
            )?;
        }
        report.observe_case(
            iteration,
            spu_observation(
                first.2.iter().copied(),
                &assessment,
                SpuTerminalObservation::from_sequence(&first.0),
                &first.0.state,
                SpuObservableSnapshot::capture(&initial),
                first.1,
                asymmetry,
            ),
        )?;
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

fn spu_observation(
    kinds: impl IntoIterator<Item = InstructionIdentity>,
    assessment: &CaseAssessment,
    terminal: SpuTerminalObservation<'_>,
    state: &SpuObservableSnapshot,
    initial_state: SpuObservableSnapshot,
    depth: u64,
    asymmetry: CrossReferenceAsymmetry,
) -> SemanticObservation {
    let kinds = kinds.into_iter().collect::<Vec<_>>();
    let outcome = terminal.outcome();
    let outcome_class = outcome.map(SpuOutcomeClass::from_outcome);
    let effects = outcome
        .into_iter()
        .flat_map(outcome_effects)
        .collect::<Vec<_>>();
    let state_changed = *state != initial_state;
    let state_transition = match outcome_class {
        Some(SpuOutcomeClass::Fault) if !state_changed => StateTransitionClass::FaultDiscarded,
        _ if !effects.is_empty() => StateTransitionClass::Effect,
        Some(SpuOutcomeClass::Branch) => StateTransitionClass::ControlFlow,
        _ if state_changed => StateTransitionClass::ArchitecturalState,
        _ => StateTransitionClass::Unchanged,
    };
    SemanticObservation {
        first_instruction_kind: kinds.first().copied(),
        instruction_kinds: kinds.into_iter().collect(),
        operands: SemanticObservation::operands_from_features(&assessment.features),
        eligibility: assessment.eligibility,
        outcome: match terminal {
            SpuTerminalObservation::DecodeRefusal(_) => Some(OutcomeIdentity::SpuDecodeRefusal),
            SpuTerminalObservation::Execution(_) => outcome_class.map(outcome_identity),
        },
        state_transition,
        effects: effects.into_iter().map(Effect::kind).collect(),
        boundaries: SemanticObservation::boundaries_from_features(&assessment.features),
        sequence_depth: depth,
        asymmetry,
    }
}

fn spu_step_replay_asymmetry(
    first: &ObservedStep,
    second: &ObservedStep,
) -> CrossReferenceAsymmetry {
    let first_observation = SpuObservation::from_parts(first.state.clone(), first.outcome.clone());
    let second_observation =
        SpuObservation::from_parts(second.state.clone(), second.outcome.clone());
    let differences = first_observation.compare(&second_observation).differences;
    replay_asymmetry(
        differences.iter().any(|component| {
            matches!(
                component,
                SpuObservationComponent::Registers
                    | SpuObservationComponent::LocalStore
                    | SpuObservationComponent::ProgramCounter
                    | SpuObservationComponent::Channels
                    | SpuObservationComponent::Reservation
                    | SpuObservationComponent::FaultDiscard
            )
        }),
        spu_outcome_asymmetry(Some(&first.outcome), Some(&second.outcome)),
        differences.contains(&SpuObservationComponent::Effects),
    )
}

fn spu_sequence_replay_asymmetry(
    first: &(ObservedSequence, u64, Vec<InstructionIdentity>),
    second: &(ObservedSequence, u64, Vec<InstructionIdentity>),
) -> CrossReferenceAsymmetry {
    let decode_refusal_differs = first.0.decode_refusal != second.0.decode_refusal;
    let state_or_trajectory_differs = first.0.state != second.0.state
        || decode_refusal_differs
        || first.0.deterministic != second.0.deterministic
        || first.0.has_undefined_operands != second.0.has_undefined_operands
        || first.0.has_unmodeled_execution != second.0.has_unmodeled_execution
        || first.0.footprint_violations != second.0.footprint_violations
        || first.1 != second.1
        || first.2 != second.2;
    let first_outcome = first.0.terminal_outcome.as_ref();
    let second_outcome = second.0.terminal_outcome.as_ref();
    let mut outcome_asymmetry = spu_outcome_asymmetry(first_outcome, second_outcome);
    if first.0.decode_refusal.is_some() != second.0.decode_refusal.is_some() {
        outcome_asymmetry = outcome_asymmetry.max(CrossReferenceAsymmetry::Outcome);
    }
    replay_asymmetry(
        state_or_trajectory_differs,
        outcome_asymmetry,
        optional_outcome_effects(first_outcome) != optional_outcome_effects(second_outcome),
    )
}

fn spu_outcome_asymmetry(
    first: Option<&SpuStepOutcome>,
    second: Option<&SpuStepOutcome>,
) -> CrossReferenceAsymmetry {
    if first == second {
        return CrossReferenceAsymmetry::None;
    }
    if [first, second].into_iter().flatten().any(|outcome| {
        spu_outcome_class_asymmetry(SpuOutcomeClass::from_outcome(outcome))
            == CrossReferenceAsymmetry::Fault
    }) {
        CrossReferenceAsymmetry::Fault
    } else {
        CrossReferenceAsymmetry::Outcome
    }
}

fn spu_outcome_class_asymmetry(outcome: SpuOutcomeClass) -> CrossReferenceAsymmetry {
    if outcome == SpuOutcomeClass::Fault {
        CrossReferenceAsymmetry::Fault
    } else {
        CrossReferenceAsymmetry::Outcome
    }
}

fn replay_asymmetry(
    state_differs: bool,
    outcome: CrossReferenceAsymmetry,
    effects_differ: bool,
) -> CrossReferenceAsymmetry {
    let mut asymmetry = CrossReferenceAsymmetry::None;
    if state_differs {
        asymmetry = CrossReferenceAsymmetry::State;
    }
    asymmetry = asymmetry.max(outcome);
    if effects_differ {
        asymmetry = asymmetry.max(CrossReferenceAsymmetry::Effect);
    }
    asymmetry
}

fn optional_outcome_effects(outcome: Option<&SpuStepOutcome>) -> &[Effect] {
    outcome.map_or(&[], outcome_effects)
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
    let mut has_undefined_operands = false;
    let mut has_unmodeled_execution = false;
    let mut footprint_violations = BTreeSet::new();
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
        has_undefined_operands |= encoding_has_undefined_operands(raw);
        has_unmodeled_execution |= !encoding_execution_is_supported(raw);
        deterministic &= requests_replay(descriptor.relations);
        decoded += 1;
        kinds.push(InstructionIdentity::Spu(descriptor.kind));
        let before = state.clone();
        let outcome = execute(&instruction, &mut state, UNIT);
        footprint_violations.extend(
            SpuAllowedFootprint::for_instruction(&instruction)
                .violations(&before, &SpuObservation::capture(&state, &outcome)),
        );
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
            has_undefined_operands,
            has_unmodeled_execution,
            footprint_violations,
        },
        decoded,
        kinds,
    )
}

fn structured_sequence(
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

fn structured_generated_word(
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

fn state_aware_state(
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

fn assess_instruction_case(
    strategy: GenerationStrategy,
    has_undefined_operands: bool,
    execution_is_supported: bool,
    descriptor: SpuFuzzDescriptor,
    outcome: &SpuStepOutcome,
    mut features: BTreeSet<CaseFeature>,
) -> CaseAssessment {
    if has_undefined_operands {
        return CaseAssessment::new(
            CaseEligibility::Undefined,
            EligibilityReason::ArchitecturallyUndefined,
            features,
        );
    }
    if !execution_is_supported {
        return CaseAssessment::new(
            CaseEligibility::Unsupported,
            EligibilityReason::UnmodeledExecution,
            features,
        );
    }
    let outcome = SpuOutcomeClass::from_outcome(outcome);
    if strategy == GenerationStrategy::Structured
        && outcome == SpuOutcomeClass::Fault
        && descriptor.outcomes.contains(&SpuOutcomeClass::Continue)
        && descriptor.outcomes.contains(&outcome)
    {
        return CaseAssessment::new(
            CaseEligibility::Unsupported,
            EligibilityReason::UnmetStatePrecondition,
            features,
        );
    }
    if strategy == GenerationStrategy::Structured && descriptor.outcomes == [SpuOutcomeClass::Fault]
    {
        features.insert(CaseFeature::NamedFaultBoundary);
        return CaseAssessment::new(
            CaseEligibility::Eligible,
            EligibilityReason::NamedFaultBoundary,
            features,
        )
        .with_reason(EligibilityReason::InterpreterContract);
    }
    let reason = match strategy {
        GenerationStrategy::Structured => EligibilityReason::StatePreconditions,
        GenerationStrategy::RawWords => EligibilityReason::DecoderRobustness,
    };
    CaseAssessment::new(CaseEligibility::Eligible, reason, features)
        .with_reason(EligibilityReason::InterpreterContract)
}

fn assess_sequence_case(
    strategy: GenerationStrategy,
    executed_depth: u64,
    has_undefined_operands: bool,
    has_unmodeled_execution: bool,
    features: BTreeSet<CaseFeature>,
) -> CaseAssessment {
    if has_undefined_operands {
        return CaseAssessment::new(
            CaseEligibility::Undefined,
            EligibilityReason::ArchitecturallyUndefined,
            features,
        );
    }
    if has_unmodeled_execution {
        return CaseAssessment::new(
            CaseEligibility::Unsupported,
            EligibilityReason::UnmodeledExecution,
            features,
        );
    }
    if strategy == GenerationStrategy::Structured && executed_depth == 0 {
        return CaseAssessment::new(
            CaseEligibility::Unsupported,
            EligibilityReason::UnmetStatePrecondition,
            features,
        );
    }
    let reason = match strategy {
        GenerationStrategy::Structured => EligibilityReason::StatePreconditions,
        GenerationStrategy::RawWords => EligibilityReason::DecoderRobustness,
    };
    CaseAssessment::new(CaseEligibility::Eligible, reason, features)
        .with_reason(EligibilityReason::InterpreterContract)
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
        config.retention,
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
#[path = "tests/spu_replay_asymmetry_tests.rs"]
mod replay_asymmetry_tests;

#[cfg(test)]
#[path = "tests/spu_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/structured_sequence_tests.rs"]
mod structured_sequence_tests;
