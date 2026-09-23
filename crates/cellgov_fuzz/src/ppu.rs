//! PPU fuzz engines built on interpreter-owned descriptors.

use std::collections::BTreeSet;

use cellgov_effects::Effect;
use cellgov_event::UnitId;
use cellgov_mem::RegionView;
use cellgov_ppu::exec::{execute, ExecuteVerdict};
use cellgov_ppu::instruction::fuzz::{
    generation_descriptors, PpuGenerationDescriptor, PpuGenerationError, PpuMetamorphicRelation,
    PpuOperandClass, PpuOutcomeClass, PpuRelationRefusal, PpuSequenceClass, PpuSequenceFlow,
};
use cellgov_ppu::instruction::PpuInstruction;
use cellgov_ppu::observation::{
    finish_observation, PpuArchitecturalState, PpuObservation, PpuObservationCheck,
    PpuObservationComponent, PpuObservationError, PpuObservationInput, PpuObservedOutcome,
};
use cellgov_ppu::state::PpuState;
use cellgov_ppu::store_buffer::StoreBuffer;
use cellgov_sync::ReservedLine;

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
const DATA_BASE: u64 = 0x1000_0000;
const DATA_REGION_BASE: u64 = DATA_BASE - 32 * 1024;
const DATA_LEN: usize = 64 * 1024;
const STRUCTURED_ENCODING_ATTEMPTS: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq)]
struct GeneratedWord {
    raw: u32,
    features: BTreeSet<CaseFeature>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct GeneratedSequence {
    words: Vec<u32>,
    features: BTreeSet<CaseFeature>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct GeneratedParameters {
    stream: ParameterStream,
    features: BTreeSet<CaseFeature>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ObservedStep {
    verdict: ExecuteVerdict,
    observation: PpuObservation,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ObservedSequence {
    observation: PpuObservation,
    terminal_verdict: Option<ExecuteVerdict>,
    decode_refusal: Option<(u64, u32)>,
    deterministic: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ObservedSequenceRun {
    observed: ObservedSequence,
    decoded: u64,
    decoded_kinds: Vec<InstructionIdentity>,
    executed: u64,
    executed_kinds: Vec<InstructionIdentity>,
}

#[derive(Clone, Copy)]
enum PpuTerminalObservation<'a> {
    Execution(Option<&'a ExecuteVerdict>),
    DecodeRefusal(Option<&'a ExecuteVerdict>),
    CommitRefusal(Option<&'a ExecuteVerdict>),
}

impl<'a> PpuTerminalObservation<'a> {
    fn from_step(observed: &'a ObservedStep) -> Self {
        if observed.observation.commit_error.is_some() {
            Self::CommitRefusal(Some(&observed.verdict))
        } else {
            Self::Execution(Some(&observed.verdict))
        }
    }

    fn from_sequence(observed: &'a ObservedSequence) -> Self {
        if observed.observation.commit_error.is_some() {
            Self::CommitRefusal(observed.terminal_verdict.as_ref())
        } else if observed.decode_refusal.is_some() {
            Self::DecodeRefusal(observed.terminal_verdict.as_ref())
        } else {
            Self::Execution(observed.terminal_verdict.as_ref())
        }
    }

    fn verdict(self) -> Option<&'a ExecuteVerdict> {
        match self {
            Self::Execution(verdict)
            | Self::DecodeRefusal(verdict)
            | Self::CommitRefusal(verdict) => verdict,
        }
    }
}

/// Checks each decoded PPU instruction against its descriptor.
pub fn run_instructions(config: FuzzConfig) -> FuzzRun {
    guarded_run(FuzzTarget::PpuInstruction, config, |report| {
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
                        CheckIdentity::PpuDecoder,
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
                            CheckIdentity::PpuDecoder,
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
        let decoded = match call_target(|| cellgov_ppu::decode::decode(raw)) {
            Ok(decoded) => decoded,
            Err(payload) => {
                record_target_panic(
                    report,
                    CheckIdentity::PpuDecoder,
                    None,
                    vec![raw],
                    iteration,
                    payload,
                )?;
                continue;
            }
        };
        let Ok(instruction) = decoded else { continue };
        let descriptor = instruction.fuzz_descriptor(raw);
        let identity = InstructionIdentity::Ppu(descriptor.kind);
        report.reached(identity)?;
        let (initial, state_features) = match config.strategy {
            GenerationStrategy::Structured => {
                state_aware_state_for_instruction(&instruction, &mut rng)?
            }
            GenerationStrategy::RawWords => (random_state(&mut rng)?, BTreeSet::new()),
        };
        let mut memory = vec![0u8; DATA_LEN];
        rng.fill(&mut memory);
        let first = match call_target(|| run_once(&instruction, &initial, &memory)) {
            Ok(Ok(first)) => first,
            Ok(Err(error)) => return Err(error.into()),
            Err(payload) => {
                record_target_panic(
                    report,
                    CheckIdentity::PpuExecutor,
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
            &instruction,
            &initial,
            descriptor,
            &first.verdict,
            features,
        );
        report.assessed(&assessment)?;
        report.executed(1)?;
        report.observed_effects(first.observation.committed_effects.iter().map(Effect::kind))?;
        let outcome = PpuOutcomeClass::from_verdict(&first.verdict);
        let mut asymmetry = CrossReferenceAsymmetry::None;
        if !descriptor.outcomes.contains(&outcome) {
            asymmetry = ppu_outcome_asymmetry(outcome);
            record(
                report,
                FindingKind::IllegalOutcome,
                SemanticFingerprint {
                    target: FuzzTarget::PpuInstruction,
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
        if let Some(effect) = first
            .observation
            .staged_effects
            .iter()
            .map(Effect::kind)
            .find(|effect| !descriptor.effects.contains(effect))
        {
            asymmetry = asymmetry.max(CrossReferenceAsymmetry::Effect);
            record(
                report,
                FindingKind::IllegalEffect,
                SemanticFingerprint {
                    target: FuzzTarget::PpuInstruction,
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
        if assessment.eligibility != CaseEligibility::Eligible {
            report.observe_case(
                iteration,
                ppu_observation(
                    [identity],
                    &assessment,
                    PpuTerminalObservation::from_step(&first),
                    &first.observation.committed_effects,
                    1,
                    ppu_step_changed(&initial, &first),
                    asymmetry,
                ),
            )?;
            continue;
        }
        if requests_replay(descriptor.relations) {
            let second = match call_target(|| run_once(&instruction, &initial, &memory)) {
                Ok(Ok(second)) => second,
                Ok(Err(error)) => return Err(error.into()),
                Err(payload) => {
                    record_target_panic(
                        report,
                        CheckIdentity::PpuExecutor,
                        Some(identity),
                        vec![raw],
                        iteration,
                        payload,
                    )?;
                    report.observe_case(
                        iteration,
                        ppu_observation(
                            [identity],
                            &assessment,
                            PpuTerminalObservation::from_step(&first),
                            &first.observation.committed_effects,
                            1,
                            ppu_step_changed(&initial, &first),
                            CrossReferenceAsymmetry::TargetPanic,
                        ),
                    )?;
                    continue;
                }
            };
            let replay_asymmetry = ppu_step_replay_asymmetry(&first, &second);
            if replay_asymmetry != CrossReferenceAsymmetry::None {
                asymmetry = asymmetry.max(replay_asymmetry);
                record(
                    report,
                    FindingKind::Nondeterministic,
                    SemanticFingerprint {
                        target: FuzzTarget::PpuInstruction,
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
        let relation_asymmetry = run_metamorphic_checks(
            report,
            MetamorphicRun {
                instruction: &instruction,
                descriptor,
                initial: &initial,
                memory: &memory,
                raw,
                identity,
                iteration,
                baseline: &first,
            },
        )?;
        asymmetry = asymmetry.max(relation_asymmetry);
        report.observe_case(
            iteration,
            ppu_observation(
                [identity],
                &assessment,
                PpuTerminalObservation::from_step(&first),
                &first.observation.committed_effects,
                1,
                ppu_step_changed(&initial, &first),
                asymmetry,
            ),
        )?;
    }
    Ok(())
}

struct MetamorphicRun<'a> {
    instruction: &'a PpuInstruction,
    descriptor: cellgov_ppu::instruction::fuzz::PpuFuzzDescriptor,
    initial: &'a PpuState,
    memory: &'a [u8],
    raw: u32,
    identity: InstructionIdentity,
    iteration: u64,
    baseline: &'a ObservedStep,
}

fn run_metamorphic_checks(
    report: &mut FuzzReport,
    run: MetamorphicRun<'_>,
) -> Result<CrossReferenceAsymmetry, FuzzError> {
    let MetamorphicRun {
        instruction,
        descriptor,
        initial,
        memory,
        raw,
        identity,
        iteration,
        baseline,
    } = run;
    if !matches!(
        baseline.verdict,
        ExecuteVerdict::Continue | ExecuteVerdict::Branch
    ) {
        for &relation in descriptor
            .relations
            .iter()
            .filter(|relation| **relation != PpuMetamorphicRelation::Deterministic)
        {
            report.metamorphic_skipped(relation_check(relation))?;
        }
        return Ok(CrossReferenceAsymmetry::None);
    }
    let mut strongest = CrossReferenceAsymmetry::None;
    for &relation in descriptor
        .relations
        .iter()
        .filter(|relation| **relation != PpuMetamorphicRelation::Deterministic)
    {
        let case = match call_target(|| instruction.metamorphic_case(raw, initial, relation)) {
            Ok(Ok(case)) => case,
            Ok(Err(
                PpuRelationRefusal::AlreadyEnabled { .. }
                | PpuRelationRefusal::IncompatibleControls { .. }
                | PpuRelationRefusal::ArchitecturallyUndefined { .. },
            )) => {
                report.metamorphic_skipped(relation_check(relation))?;
                continue;
            }
            Ok(Err(
                PpuRelationRefusal::Undeclared { .. } | PpuRelationRefusal::InvalidPartner { .. },
            )) => {
                record_invalid_metamorphic_partner(
                    report,
                    relation,
                    identity,
                    DivergenceClass::ReferenceDisagreement,
                    None,
                    vec![raw],
                    iteration,
                )?;
                strongest = strongest.max(CrossReferenceAsymmetry::Outcome);
                continue;
            }
            Err(payload) => {
                record_target_panic(
                    report,
                    relation_check(relation),
                    Some(identity),
                    vec![raw],
                    iteration,
                    payload,
                )?;
                strongest = strongest.max(CrossReferenceAsymmetry::TargetPanic);
                continue;
            }
        };
        let partner_instruction =
            match call_target(|| cellgov_ppu::decode::decode(case.partner_word)) {
                Ok(Ok(instruction)) => instruction,
                Ok(Err(_)) => {
                    record_invalid_metamorphic_partner(
                        report,
                        relation,
                        identity,
                        DivergenceClass::Outcome,
                        Some(OutcomeIdentity::PpuDecodeRefusal),
                        vec![raw, case.partner_word],
                        iteration,
                    )?;
                    strongest = strongest.max(CrossReferenceAsymmetry::Outcome);
                    continue;
                }
                Err(payload) => {
                    record_target_panic(
                        report,
                        relation_check(relation),
                        Some(identity),
                        vec![raw, case.partner_word],
                        iteration,
                        payload,
                    )?;
                    strongest = strongest.max(CrossReferenceAsymmetry::TargetPanic);
                    continue;
                }
            };
        let partner = match call_target(|| run_once(&partner_instruction, initial, memory)) {
            Ok(Ok(observed)) => observed,
            Ok(Err(error)) => return Err(error.into()),
            Err(payload) => {
                record_target_panic(
                    report,
                    relation_check(relation),
                    Some(identity),
                    vec![raw, case.partner_word],
                    iteration,
                    payload,
                )?;
                strongest = strongest.max(CrossReferenceAsymmetry::TargetPanic);
                continue;
            }
        };
        report.metamorphic_executed(relation_check(relation))?;
        let comparison = baseline
            .observation
            .compare_metamorphic(&partner.observation, case.permitted_delta);
        if comparison.disallowed_differences.is_empty() {
            continue;
        }
        let (divergence, asymmetry) = metamorphic_divergence(&comparison.disallowed_differences);
        strongest = strongest.max(asymmetry);
        record(
            report,
            FindingKind::MetamorphicViolation,
            SemanticFingerprint {
                target: FuzzTarget::PpuInstruction,
                instruction_kind: Some(identity),
                check: relation_check(relation),
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

fn relation_check(relation: PpuMetamorphicRelation) -> CheckIdentity {
    match relation {
        PpuMetamorphicRelation::Deterministic => CheckIdentity::DeterministicReplay,
        PpuMetamorphicRelation::RecordCr0 => CheckIdentity::PpuRecordCr0,
        PpuMetamorphicRelation::RecordCr1 => CheckIdentity::PpuRecordCr1,
        PpuMetamorphicRelation::RecordCr6 => CheckIdentity::PpuRecordCr6,
        PpuMetamorphicRelation::OverflowEnable => CheckIdentity::PpuOverflowEnable,
    }
}

fn metamorphic_divergence(
    differences: &BTreeSet<PpuObservationComponent>,
) -> (DivergenceClass, CrossReferenceAsymmetry) {
    if differences.contains(&PpuObservationComponent::FaultDiscard) {
        (DivergenceClass::Outcome, CrossReferenceAsymmetry::Fault)
    } else if differences.contains(&PpuObservationComponent::StagedEffects)
        || differences.contains(&PpuObservationComponent::CommittedEffects)
    {
        (DivergenceClass::Effect, CrossReferenceAsymmetry::Effect)
    } else if differences.contains(&PpuObservationComponent::Outcome) {
        (DivergenceClass::Outcome, CrossReferenceAsymmetry::Outcome)
    } else {
        (
            DivergenceClass::ArchitecturalState,
            CrossReferenceAsymmetry::State,
        )
    }
}

/// Replays PPU instruction sequences to find nondeterministic outcomes.
pub fn run_sequences(config: FuzzConfig) -> FuzzRun {
    guarded_run(FuzzTarget::PpuSequence, config, |report| {
        run_sequences_inner(config, report)
    })
}

fn run_sequences_inner(config: FuzzConfig, report: &mut FuzzReport) -> Result<(), FuzzError> {
    config.validate(Some(crate::MAX_SEQUENCE_WORDS))?;
    let iterations = config.case_indices()?;
    let descriptors = match (config.strategy, iterations.clone().next()) {
        (GenerationStrategy::Structured, Some(first)) => {
            match call_target(generation_descriptors) {
                Ok(descriptors) => descriptors,
                Err(payload) => {
                    report.considered()?;
                    record_target_panic(
                        report,
                        CheckIdentity::PpuDecoder,
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
                    call_target(|| cellgov_ppu::decode::decode(raw)).map(|decoded| decoded.is_ok())
                })
                .map(|words| GeneratedSequence {
                    words,
                    features: BTreeSet::new(),
                }),
        };
        let generated = match generated {
            Ok(generated) => generated,
            Err(WordGenerationFailure::DecoderPanic(raw, payload)) => {
                record_target_panic(
                    report,
                    CheckIdentity::PpuDecoder,
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
        let (mut initial, state_features) =
            state_aware_state_for_sequence(config.strategy, &mut rng)?;
        initial.pc = 0;
        let mut memory = vec![0u8; DATA_LEN];
        rng.fill(&mut memory);
        if words.is_empty() {
            return Err(InvariantError::EmptyGeneratedSequence.into());
        }
        let first = match call_target(|| run_sequence(&words, &initial, &memory)) {
            Ok(Ok(first)) => first,
            Ok(Err(error)) => return Err(error.into()),
            Err(payload) => {
                record_target_panic(
                    report,
                    CheckIdentity::PpuExecutor,
                    None,
                    words.clone(),
                    iteration,
                    payload,
                )?;
                continue;
            }
        };
        report.reached_many(first.decoded, first.decoded_kinds.iter().copied())?;
        let mut features = generated.features;
        features.extend(state_features);
        let assessment = assess_sequence_case(config.strategy, first.executed, features);
        report.assessed(&assessment)?;
        report.executed(first.executed)?;
        report.observed_effects(
            first
                .observed
                .observation
                .committed_effects
                .iter()
                .map(Effect::kind),
        )?;
        if assessment.eligibility != CaseEligibility::Eligible {
            report.observe_case(
                iteration,
                ppu_observation(
                    first.executed_kinds.iter().copied(),
                    &assessment,
                    PpuTerminalObservation::from_sequence(&first.observed),
                    &first.observed.observation.committed_effects,
                    first.executed,
                    ppu_sequence_changed(&initial, &first.observed),
                    CrossReferenceAsymmetry::None,
                ),
            )?;
            continue;
        }
        let mut asymmetry = CrossReferenceAsymmetry::None;
        if first.observed.deterministic {
            let second = match call_target(|| run_sequence(&words, &initial, &memory)) {
                Ok(Ok(second)) => second,
                Ok(Err(error)) => return Err(error.into()),
                Err(payload) => {
                    record_target_panic(
                        report,
                        CheckIdentity::PpuExecutor,
                        None,
                        words.clone(),
                        iteration,
                        payload,
                    )?;
                    report.observe_case(
                        iteration,
                        ppu_observation(
                            first.executed_kinds.iter().copied(),
                            &assessment,
                            PpuTerminalObservation::from_sequence(&first.observed),
                            &first.observed.observation.committed_effects,
                            first.executed,
                            ppu_sequence_changed(&initial, &first.observed),
                            CrossReferenceAsymmetry::TargetPanic,
                        ),
                    )?;
                    continue;
                }
            };
            let replay_asymmetry = ppu_sequence_replay_asymmetry(&first, &second);
            if replay_asymmetry != CrossReferenceAsymmetry::None {
                asymmetry = asymmetry.max(replay_asymmetry);
                record(
                    report,
                    FindingKind::Nondeterministic,
                    SemanticFingerprint {
                        target: FuzzTarget::PpuSequence,
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
        report.observe_case(
            iteration,
            ppu_observation(
                first.executed_kinds.iter().copied(),
                &assessment,
                PpuTerminalObservation::from_sequence(&first.observed),
                &first.observed.observation.committed_effects,
                first.executed,
                ppu_sequence_changed(&initial, &first.observed),
                asymmetry,
            ),
        )?;
    }
    Ok(())
}

fn run_once(
    instruction: &PpuInstruction,
    initial: &PpuState,
    memory: &[u8],
) -> Result<ObservedStep, PpuObservationError> {
    let mut state = initial.clone();
    let mut effects = Vec::new();
    let mut stores = StoreBuffer::new();
    let views = [RegionView::plain(DATA_REGION_BASE, memory)];
    let verdict = execute(
        instruction,
        &mut state,
        UNIT,
        &views,
        &mut effects,
        &mut stores,
    );
    let observation = finish_fuzz_observation(PpuObservationInput {
        initial_state: initial,
        final_state: &state,
        memory_base: DATA_REGION_BASE,
        initial_memory: memory,
        outcome: PpuObservedOutcome::Execution(verdict.clone()),
        effects,
        stores,
        unit: UNIT,
    })?;
    Ok(ObservedStep {
        verdict,
        observation,
    })
}

fn finish_fuzz_observation(
    input: PpuObservationInput<'_>,
) -> Result<PpuObservation, PpuObservationError> {
    finish_observation(input)
}

fn ppu_observation(
    kinds: impl IntoIterator<Item = InstructionIdentity>,
    assessment: &CaseAssessment,
    terminal: PpuTerminalObservation<'_>,
    committed_effects: &[Effect],
    depth: u64,
    state_changed: bool,
    asymmetry: CrossReferenceAsymmetry,
) -> SemanticObservation {
    let kinds = kinds.into_iter().collect::<Vec<_>>();
    let verdict = terminal.verdict();
    let outcome = verdict.map(PpuOutcomeClass::from_verdict);
    let state_transition = match (terminal, outcome) {
        (PpuTerminalObservation::DecodeRefusal(_), _) => StateTransitionClass::FaultDiscarded,
        (PpuTerminalObservation::CommitRefusal(_), _) => StateTransitionClass::CommitRefused,
        (
            PpuTerminalObservation::Execution(_),
            Some(PpuOutcomeClass::Fault | PpuOutcomeClass::MemoryFault),
        ) if !state_changed => StateTransitionClass::FaultDiscarded,
        _ if !committed_effects.is_empty() => StateTransitionClass::Effect,
        (_, Some(PpuOutcomeClass::Branch)) => StateTransitionClass::ControlFlow,
        _ if state_changed => StateTransitionClass::ArchitecturalState,
        _ => StateTransitionClass::Unchanged,
    };
    SemanticObservation {
        first_instruction_kind: kinds.first().copied(),
        instruction_kinds: kinds.into_iter().collect(),
        operands: SemanticObservation::operands_from_features(&assessment.features),
        eligibility: assessment.eligibility,
        outcome: match terminal {
            PpuTerminalObservation::DecodeRefusal(_) => Some(OutcomeIdentity::PpuDecodeRefusal),
            PpuTerminalObservation::CommitRefusal(_) => Some(OutcomeIdentity::PpuCommitRefusal),
            PpuTerminalObservation::Execution(_) => outcome.map(outcome_identity),
        },
        state_transition,
        effects: committed_effects.iter().map(Effect::kind).collect(),
        boundaries: SemanticObservation::boundaries_from_features(&assessment.features),
        sequence_depth: depth,
        asymmetry,
    }
}

fn ppu_step_changed(initial: &PpuState, observed: &ObservedStep) -> bool {
    observed.observation.state != PpuArchitecturalState::capture(initial)
}

fn ppu_sequence_changed(initial: &PpuState, observed: &ObservedSequence) -> bool {
    observed.observation.state != PpuArchitecturalState::capture(initial)
}

fn ppu_step_replay_asymmetry(
    first: &ObservedStep,
    second: &ObservedStep,
) -> CrossReferenceAsymmetry {
    let comparison = first.observation.compare(
        &second.observation,
        PpuObservationCheck::DeterministicReplay,
    );
    let state_differs = comparison.relevant_differences.iter().any(|component| {
        matches!(
            component,
            PpuObservationComponent::State
                | PpuObservationComponent::Memory
                | PpuObservationComponent::Reservations
                | PpuObservationComponent::StoreBuffer
                | PpuObservationComponent::FaultDiscard
        )
    });
    let mut outcome_asymmetry =
        ppu_observed_outcome_asymmetry(&first.observation.outcome, &second.observation.outcome)
            .max(ppu_verdict_asymmetry(
                Some(&first.verdict),
                Some(&second.verdict),
            ));
    if comparison
        .relevant_differences
        .contains(&PpuObservationComponent::Outcome)
    {
        outcome_asymmetry = outcome_asymmetry.max(CrossReferenceAsymmetry::Outcome);
    }
    replay_asymmetry(
        state_differs,
        outcome_asymmetry,
        comparison.relevant_differences.iter().any(|component| {
            matches!(
                component,
                PpuObservationComponent::StagedEffects | PpuObservationComponent::CommittedEffects
            )
        }),
    )
}

fn ppu_sequence_replay_asymmetry(
    first: &ObservedSequenceRun,
    second: &ObservedSequenceRun,
) -> CrossReferenceAsymmetry {
    let decode_refusal_differs = first.observed.decode_refusal != second.observed.decode_refusal;
    let comparison = first.observed.observation.compare(
        &second.observed.observation,
        PpuObservationCheck::DeterministicReplay,
    );
    let state_or_trajectory_differs = comparison.relevant_differences.iter().any(|component| {
        matches!(
            component,
            PpuObservationComponent::State
                | PpuObservationComponent::Memory
                | PpuObservationComponent::Reservations
                | PpuObservationComponent::StoreBuffer
                | PpuObservationComponent::FaultDiscard
        )
    }) || decode_refusal_differs
        || first.observed.deterministic != second.observed.deterministic
        || first.decoded != second.decoded
        || first.decoded_kinds != second.decoded_kinds
        || first.executed != second.executed
        || first.executed_kinds != second.executed_kinds;
    let mut outcome_asymmetry = ppu_observed_outcome_asymmetry(
        &first.observed.observation.outcome,
        &second.observed.observation.outcome,
    )
    .max(ppu_verdict_asymmetry(
        first.observed.terminal_verdict.as_ref(),
        second.observed.terminal_verdict.as_ref(),
    ));
    if comparison
        .relevant_differences
        .contains(&PpuObservationComponent::Outcome)
        || first.observed.decode_refusal.is_some() != second.observed.decode_refusal.is_some()
    {
        outcome_asymmetry = outcome_asymmetry.max(CrossReferenceAsymmetry::Outcome);
    }
    replay_asymmetry(
        state_or_trajectory_differs,
        outcome_asymmetry,
        comparison.relevant_differences.iter().any(|component| {
            matches!(
                component,
                PpuObservationComponent::StagedEffects | PpuObservationComponent::CommittedEffects
            )
        }),
    )
}

fn ppu_verdict_asymmetry(
    first: Option<&ExecuteVerdict>,
    second: Option<&ExecuteVerdict>,
) -> CrossReferenceAsymmetry {
    if first == second {
        return CrossReferenceAsymmetry::None;
    }
    if [first, second].into_iter().flatten().any(|verdict| {
        ppu_outcome_asymmetry(PpuOutcomeClass::from_verdict(verdict))
            == CrossReferenceAsymmetry::Fault
    }) {
        CrossReferenceAsymmetry::Fault
    } else {
        CrossReferenceAsymmetry::Outcome
    }
}

fn ppu_observed_outcome_asymmetry(
    first: &PpuObservedOutcome,
    second: &PpuObservedOutcome,
) -> CrossReferenceAsymmetry {
    if first == second {
        return CrossReferenceAsymmetry::None;
    }
    if [first, second].into_iter().any(|outcome| {
        matches!(
            outcome,
            PpuObservedOutcome::Execution(verdict)
                if ppu_outcome_asymmetry(PpuOutcomeClass::from_verdict(verdict))
                    == CrossReferenceAsymmetry::Fault
        )
    }) {
        CrossReferenceAsymmetry::Fault
    } else {
        CrossReferenceAsymmetry::Outcome
    }
}

fn ppu_outcome_asymmetry(outcome: PpuOutcomeClass) -> CrossReferenceAsymmetry {
    if matches!(
        outcome,
        PpuOutcomeClass::Fault | PpuOutcomeClass::MemoryFault
    ) {
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

fn run_sequence(
    words: &[u32],
    initial: &PpuState,
    memory: &[u8],
) -> Result<ObservedSequenceRun, PpuObservationError> {
    let mut state = initial.clone();
    let mut effects = Vec::new();
    let mut stores = StoreBuffer::new();
    let views = [RegionView::plain(DATA_REGION_BASE, memory)];
    let mut decoded = 0u64;
    let mut decoded_kinds = Vec::new();
    let mut executed = 0u64;
    let mut executed_kinds = Vec::new();
    let mut terminal_verdict = None;
    let mut decode_refusal = None;
    let mut deterministic = true;
    for _ in 0..words.len() {
        // Mirror `PpuExecutionUnit::run_batch`: a branch sets the PC for the next fetched word.
        let Some(index) = state
            .pc
            .checked_div(4)
            .and_then(|index| usize::try_from(index).ok())
        else {
            break;
        };
        let Some(&raw) = words.get(index) else {
            break;
        };
        let Ok(instruction) = cellgov_ppu::decode::decode(raw) else {
            decode_refusal = Some((state.pc, raw));
            break;
        };
        let descriptor = instruction.fuzz_descriptor(raw);
        deterministic &= requests_replay(descriptor.relations);
        decoded += 1;
        let identity = InstructionIdentity::Ppu(descriptor.kind);
        decoded_kinds.push(identity);
        let verdict = execute(
            &instruction,
            &mut state,
            UNIT,
            &views,
            &mut effects,
            &mut stores,
        );
        terminal_verdict = Some(verdict.clone());
        // The runtime retries `BufferFull`.
        if verdict != ExecuteVerdict::BufferFull {
            executed += 1;
            executed_kinds.push(identity);
        }
        match verdict {
            ExecuteVerdict::Continue => state.pc = state.pc.wrapping_add(4),
            ExecuteVerdict::Branch => {}
            ExecuteVerdict::Fault(_) | ExecuteVerdict::MemFault(_) => {
                break;
            }
            ExecuteVerdict::Syscall { .. } | ExecuteVerdict::BufferFull => break,
        }
    }
    let outcome = match decode_refusal {
        Some((pc, raw)) => PpuObservedOutcome::DecodeRefusal {
            pc,
            raw,
            prior: terminal_verdict.clone(),
        },
        None => terminal_verdict
            .clone()
            .map(PpuObservedOutcome::Execution)
            .unwrap_or(PpuObservedOutcome::NoInstruction),
    };
    let observation = finish_fuzz_observation(PpuObservationInput {
        initial_state: initial,
        final_state: &state,
        memory_base: DATA_REGION_BASE,
        initial_memory: memory,
        outcome,
        effects,
        stores,
        unit: UNIT,
    })?;
    Ok(ObservedSequenceRun {
        observed: ObservedSequence {
            observation,
            terminal_verdict,
            decode_refusal,
            deterministic,
        },
        decoded,
        decoded_kinds,
        executed,
        executed_kinds,
    })
}

#[cfg(test)]
fn structured_words(
    descriptors: &[PpuGenerationDescriptor],
    rng: &mut Rng,
    count: usize,
) -> Result<Vec<u32>, GeneratorError> {
    structured_sequence(descriptors, rng, count).map(|generated| generated.words)
}

fn structured_sequence(
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
    // [Wang2024 p:340:1 s:Abstract] Generated programs track program state statically.
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

fn structured_generated_word(
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
    // [Yang2011 p:1 s:Abstract] Only valid typed operand combinations reach comparison.
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

fn random_state(rng: &mut Rng) -> Result<PpuState, GeneratorError> {
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

fn state_aware_state_for_instruction(
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

fn state_aware_state_for_sequence(
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

fn assess_instruction_case(
    strategy: GenerationStrategy,
    instruction: &PpuInstruction,
    initial: &PpuState,
    descriptor: cellgov_ppu::instruction::fuzz::PpuFuzzDescriptor,
    verdict: &ExecuteVerdict,
    mut features: BTreeSet<CaseFeature>,
) -> CaseAssessment {
    if instruction.fuzz_case_is_architecturally_undefined(initial) {
        return CaseAssessment::new(
            CaseEligibility::Undefined,
            EligibilityReason::ArchitecturallyUndefined,
            features,
        );
    }
    let outcome = PpuOutcomeClass::from_verdict(verdict);
    if strategy == GenerationStrategy::Structured
        && matches!(
            outcome,
            PpuOutcomeClass::MemoryFault | PpuOutcomeClass::Fault
        )
        && descriptor.outcomes.contains(&PpuOutcomeClass::Continue)
        && descriptor.outcomes.contains(&outcome)
    {
        if outcome == PpuOutcomeClass::MemoryFault {
            features.remove(&CaseFeature::MappedMemory);
            features.remove(&CaseFeature::Reservation);
        }
        return CaseAssessment::new(
            CaseEligibility::Unsupported,
            EligibilityReason::UnmetStatePrecondition,
            features,
        );
    }
    if strategy == GenerationStrategy::Structured && descriptor.outcomes == [PpuOutcomeClass::Fault]
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
    features: BTreeSet<CaseFeature>,
) -> CaseAssessment {
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

fn requests_replay(relations: &[PpuMetamorphicRelation]) -> bool {
    relations.contains(&PpuMetamorphicRelation::Deterministic)
}

fn outcome_identity(outcome: PpuOutcomeClass) -> OutcomeIdentity {
    match outcome {
        PpuOutcomeClass::Continue => OutcomeIdentity::PpuContinue,
        PpuOutcomeClass::Branch => OutcomeIdentity::PpuBranch,
        PpuOutcomeClass::Syscall => OutcomeIdentity::PpuSyscall,
        PpuOutcomeClass::Fault => OutcomeIdentity::PpuFault,
        PpuOutcomeClass::MemoryFault => OutcomeIdentity::PpuMemoryFault,
        PpuOutcomeClass::BufferFull => OutcomeIdentity::PpuBufferFull,
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
        observation: None,
        reduction: ReductionOutcome::NotAttempted,
        panic_payload: None,
    })
}

fn record_invalid_metamorphic_partner(
    report: &mut FuzzReport,
    relation: PpuMetamorphicRelation,
    instruction_kind: InstructionIdentity,
    divergence: DivergenceClass,
    outcome: Option<OutcomeIdentity>,
    original_words: Vec<u32>,
    iteration: u64,
) -> Result<(), InvariantError> {
    record(
        report,
        FindingKind::MetamorphicViolation,
        SemanticFingerprint {
            target: FuzzTarget::PpuInstruction,
            instruction_kind: Some(instruction_kind),
            check: relation_check(relation),
            divergence,
            outcome,
            effect: None,
        },
        original_words,
        iteration,
    )
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
        observation: None,
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
                stage: "PPU campaign",
            },
        ),
    }
}

#[cfg(test)]
#[path = "tests/ppu_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/ppu_metamorphic_tests.rs"]
mod metamorphic_tests;
