//! The PPU instruction engine: one decoded instruction per case, checked
//! against its descriptor and its metamorphic relations.

use std::collections::BTreeSet;

use cellgov_effects::Effect;
use cellgov_ppu::exec::ExecuteVerdict;
use cellgov_ppu::instruction::fuzz::{
    generation_descriptors, PpuMetamorphicRelation, PpuOutcomeClass, PpuRelationRefusal,
};
use cellgov_ppu::instruction::PpuInstruction;
use cellgov_ppu::observation::PpuObservationComponent;
use cellgov_ppu::state::PpuState;

use super::assess::assess_instruction_case;
use super::execute::{
    outcome_identity, ppu_observation, ppu_outcome_asymmetry, ppu_step_changed,
    ppu_step_replay_asymmetry, requests_replay, run_once, ObservedStep, PpuTerminalObservation,
};
use super::generate::{
    case_descriptors, random_state, state_aware_state_for_instruction, structured_generated_word,
    GeneratedWord, DATA_LEN,
};
use super::record::{guarded_run, record, record_target_panic};
use crate::boundary::call_target;
use crate::case::CaseEligibility;
use crate::error::{FuzzError, InvariantError};
use crate::report::{
    CheckIdentity, DivergenceClass, FindingKind, FuzzReport, FuzzRun, FuzzTarget,
    InstructionIdentity, OutcomeIdentity, SemanticFingerprint,
};
use crate::retention::CrossReferenceAsymmetry;
use crate::rng::Rng;
use crate::seeded;
use crate::{FuzzConfig, GenerationStrategy};

/// Checks each decoded PPU instruction against its descriptor.
pub fn run_instructions(config: FuzzConfig) -> FuzzRun {
    run_instructions_with(config, None)
}

/// Runs the instruction engine; `words` replaces every case's generated word.
pub(crate) fn run_instructions_with(config: FuzzConfig, words: Option<&[u32]>) -> FuzzRun {
    guarded_run(FuzzTarget::PpuInstruction, config, |report| {
        run_instructions_inner(config, report, words)
    })
}

/// Generator words for one instruction case, before any check runs.
pub(crate) fn instruction_case_words(
    config: FuzzConfig,
    case_index: u64,
) -> Result<Vec<u32>, FuzzError> {
    config.validate(None)?;
    let descriptors = case_descriptors(config)?;
    let mut rng = Rng::for_case(config.campaign_version, config.seed, case_index);
    let raw = match config.strategy {
        GenerationStrategy::Structured => {
            call_target(|| structured_generated_word(&descriptors, &mut rng, None))
                .map_err(|_| InvariantError::UnexpectedPanic {
                    stage: "PPU case generation",
                })??
                .raw
        }
        GenerationStrategy::RawWords => rng.next_u32(),
    };
    Ok(vec![raw])
}

fn run_instructions_inner(
    config: FuzzConfig,
    report: &mut FuzzReport,
    override_words: Option<&[u32]>,
) -> Result<(), FuzzError> {
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
        // The generator draws first so a substituted word keeps the case's state.
        let raw = override_words
            .and_then(|words| words.first().copied())
            .unwrap_or(generated.raw);
        let decoded = match call_target(|| seeded::ppu_decode(raw)) {
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
        // An unsupported or undefined case enters no check, so it produces no finding.
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
                    CrossReferenceAsymmetry::None,
                ),
            )?;
            continue;
        }
        let outcome = seeded::ppu_outcome(
            PpuOutcomeClass::from_verdict(&first.verdict),
            descriptor.outcomes,
        );
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
            .chain(seeded::extra_effect(descriptor.effects))
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
        if requests_replay(descriptor.relations) {
            let second = match call_target(|| run_once(&instruction, &initial, &memory)) {
                Ok(Ok(mut second)) => {
                    seeded::ppu_replayed(&mut second.observation);
                    second
                }
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

// [Le2014 p:219 s:3.1.2] Each equivalent variant runs on the same input as the original, and any disagreement is a finding.
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
        let partner_instruction = match call_target(|| seeded::ppu_decode(case.partner_word)) {
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
            Ok(Ok(mut observed)) => {
                seeded::ppu_partner(&mut observed.observation);
                observed
            }
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

#[cfg(test)]
#[path = "tests/instructions_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/store_wrap_tests.rs"]
mod store_wrap_tests;

#[cfg(test)]
#[path = "tests/record_cr6_tests.rs"]
mod record_cr6_tests;
