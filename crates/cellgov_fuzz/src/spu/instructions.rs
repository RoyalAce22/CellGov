//! The SPU instruction engine: one decoded instruction per case, checked
//! against its descriptor and its metamorphic relations.

use std::collections::BTreeSet;

use cellgov_effects::Effect;
use cellgov_spu::fuzz::{
    encoding_execution_is_supported, encoding_has_undefined_operands, generation_descriptors,
    SpuFuzzDescriptor, SpuMetamorphicRelation, SpuOutcomeClass, SpuRelationRefusal,
};
use cellgov_spu::observation::{SpuAllowedFootprint, SpuObservation, SpuObservationComponent};
use cellgov_spu::state::{SpuObservableSnapshot, SpuState};

use super::assess::assess_instruction_case;
use super::execute::{
    outcome_effects, outcome_identity, requests_replay, run_once, spu_observation,
    spu_outcome_class_asymmetry, spu_step_replay_asymmetry, ObservedStep, SpuTerminalObservation,
};
use super::generate::{
    case_descriptors, random_state, state_aware_state, structured_generated_word, GeneratedWord,
};
use super::record::{guarded_run, record, record_target_panic};
use crate::boundary::call_target;
use crate::case::CaseEligibility;
use crate::error::{FuzzError, InvariantError};
use crate::report::{
    CheckIdentity, DivergenceClass, FindingKind, FuzzReport, FuzzRun, FuzzTarget,
    InstructionIdentity, SemanticFingerprint,
};
use crate::retention::CrossReferenceAsymmetry;
use crate::rng::Rng;
use crate::seeded;
use crate::{FuzzConfig, GenerationStrategy};

/// Checks each decoded SPU instruction against its descriptor.
pub fn run_instructions(config: FuzzConfig) -> FuzzRun {
    run_instructions_with(config, None)
}

/// Runs the instruction engine; `words` replaces every case's generated word.
pub(crate) fn run_instructions_with(config: FuzzConfig, words: Option<&[u32]>) -> FuzzRun {
    guarded_run(FuzzTarget::SpuInstruction, config, |report| {
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
                    stage: "SPU case generation",
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
        // The generator draws first so a substituted word keeps the case's state.
        let raw = override_words
            .and_then(|words| words.first().copied())
            .unwrap_or(generated.raw);
        let decoded = match call_target(|| seeded::spu_decode(raw)) {
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
                Ok(mut second) => {
                    seeded::spu_replayed(&mut second.state);
                    second
                }
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
        let outcome = seeded::spu_outcome(
            SpuOutcomeClass::from_outcome(&first.outcome),
            descriptor.outcomes,
        );
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
            .chain(seeded::extra_effect(descriptor.effects))
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
            let decoded = seeded::spu_decode(case.partner_word);
            decoded.map(|instruction| run_once(&instruction, initial))
        }) {
            Ok(Ok(mut partner)) => {
                seeded::spu_partner(&mut partner.state);
                partner
            }
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

#[cfg(test)]
#[path = "tests/instructions_tests.rs"]
mod tests;
