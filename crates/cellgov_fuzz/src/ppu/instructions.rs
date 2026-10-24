//! The PPU instruction engine: one decoded instruction per case, checked
//! against its descriptor and its metamorphic relations.

use std::collections::BTreeSet;

use cellgov_effects::{Effect, EffectKind};
use cellgov_ppu::exec::ExecuteVerdict;
use cellgov_ppu::instruction::fuzz::{PpuMetamorphicRelation, PpuOutcomeClass, PpuRelationRefusal};
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
use super::record::{guarded_run, load_descriptors, record, record_target_panic};
use crate::boundary::call_target;
use crate::case::{CaseAssessment, CaseEligibility};
use crate::error::{FuzzError, InvariantError};
use crate::report::{
    CheckIdentity, ComponentIdentity, DivergenceClass, FindingKind, FuzzReport, FuzzRun,
    FuzzTarget, InstructionIdentity, OutcomeIdentity, SemanticFingerprint,
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
    let Some(descriptors) = load_descriptors(config.strategy, iterations.clone().next(), report)?
    else {
        return Ok(());
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
        let case = InstructionCase {
            run: MetamorphicRun {
                instruction: &instruction,
                descriptor,
                initial: &initial,
                memory: &memory,
                raw,
                identity,
                iteration,
                baseline: &first,
            },
            assessment: &assessment,
        };
        // An unsupported or undefined case enters no check, so it produces no finding.
        if assessment.eligibility != CaseEligibility::Eligible {
            case.observe(report, CrossReferenceAsymmetry::None)?;
            continue;
        }
        let mut asymmetry = case.check_legal_outcome(report)?;
        asymmetry = asymmetry.max(case.check_legal_effect(report)?);
        let Some(replay_asymmetry) = case.check_replay(report)? else {
            case.observe(report, CrossReferenceAsymmetry::TargetPanic)?;
            continue;
        };
        asymmetry = asymmetry.max(replay_asymmetry);
        asymmetry = asymmetry.max(run_metamorphic_checks(report, case.run)?);
        case.observe(report, asymmetry)?;
    }
    Ok(())
}

/// One decoded instruction case after its first run, and the checks an
/// eligible case enters. Each check records its finding and returns the
/// asymmetry the case's observation takes.
struct InstructionCase<'a> {
    run: MetamorphicRun<'a>,
    assessment: &'a CaseAssessment,
}

impl InstructionCase<'_> {
    fn observe(
        &self,
        report: &mut FuzzReport,
        asymmetry: CrossReferenceAsymmetry,
    ) -> Result<(), InvariantError> {
        let run = &self.run;
        report.observe_case(
            run.iteration,
            ppu_observation(
                [run.identity],
                self.assessment,
                PpuTerminalObservation::from_step(run.baseline),
                &run.baseline.observation.committed_effects,
                1,
                ppu_step_changed(run.initial, run.baseline),
                asymmetry,
            ),
        )?;
        Ok(())
    }

    fn fingerprint(
        &self,
        check: CheckIdentity,
        divergence: DivergenceClass,
        outcome: Option<OutcomeIdentity>,
        effect: Option<EffectKind>,
    ) -> SemanticFingerprint {
        SemanticFingerprint {
            target: FuzzTarget::PpuInstruction,
            instruction_kind: Some(self.run.identity),
            check,
            divergence,
            outcome,
            effect,
            component: None,
        }
    }

    fn check_legal_outcome(
        &self,
        report: &mut FuzzReport,
    ) -> Result<CrossReferenceAsymmetry, InvariantError> {
        let run = &self.run;
        let outcome = seeded::ppu_outcome(
            PpuOutcomeClass::from_verdict(&run.baseline.verdict),
            run.descriptor.outcomes,
        );
        if run.descriptor.outcomes.contains(&outcome) {
            return Ok(CrossReferenceAsymmetry::None);
        }
        record(
            report,
            FindingKind::IllegalOutcome,
            self.fingerprint(
                CheckIdentity::LegalOutcome,
                DivergenceClass::Outcome,
                Some(outcome_identity(outcome)),
                None,
            ),
            vec![run.raw],
            run.iteration,
        )?;
        Ok(ppu_outcome_asymmetry(outcome))
    }

    fn check_legal_effect(
        &self,
        report: &mut FuzzReport,
    ) -> Result<CrossReferenceAsymmetry, InvariantError> {
        let run = &self.run;
        let Some(effect) = run
            .baseline
            .observation
            .staged_effects
            .iter()
            .map(Effect::kind)
            .chain(seeded::extra_effect(run.descriptor.effects))
            .find(|effect| !run.descriptor.effects.contains(effect))
        else {
            return Ok(CrossReferenceAsymmetry::None);
        };
        record(
            report,
            FindingKind::IllegalEffect,
            self.fingerprint(
                CheckIdentity::LegalEffect,
                DivergenceClass::Effect,
                None,
                Some(effect),
            ),
            vec![run.raw],
            run.iteration,
        )?;
        Ok(CrossReferenceAsymmetry::Effect)
    }

    /// The replay's asymmetry, `None` asymmetry for a kind that asks for
    /// no replay; `Ok(None)` when the replay panicked, recorded here.
    fn check_replay(
        &self,
        report: &mut FuzzReport,
    ) -> Result<Option<CrossReferenceAsymmetry>, FuzzError> {
        let run = &self.run;
        if !requests_replay(run.descriptor.relations) {
            return Ok(Some(CrossReferenceAsymmetry::None));
        }
        let second = match call_target(|| run_once(run.instruction, run.initial, run.memory)) {
            Ok(Ok(mut second)) => {
                seeded::ppu_replayed(&mut second.observation);
                second
            }
            Ok(Err(error)) => return Err(error.into()),
            Err(payload) => {
                record_target_panic(
                    report,
                    CheckIdentity::PpuExecutor,
                    Some(run.identity),
                    vec![run.raw],
                    run.iteration,
                    payload,
                )?;
                return Ok(None);
            }
        };
        let replay_asymmetry = ppu_step_replay_asymmetry(run.baseline, &second);
        if replay_asymmetry != CrossReferenceAsymmetry::None {
            record(
                report,
                FindingKind::Nondeterministic,
                self.fingerprint(
                    CheckIdentity::DeterministicReplay,
                    DivergenceClass::ArchitecturalState,
                    None,
                    None,
                ),
                vec![run.raw],
                run.iteration,
            )?;
        }
        Ok(Some(replay_asymmetry))
    }
}

#[derive(Clone, Copy)]
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

/// [Le2014 p:219 s:3.1.2] Each equivalent variant runs on the same input as the original, and any disagreement is a finding.
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
        let Some((component, divergence, asymmetry)) =
            metamorphic_divergence(&comparison.disallowed_differences)
        else {
            continue;
        };
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
                component: Some(ComponentIdentity::Ppu(component)),
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

/// The component that decides a metamorphic finding's class, with that
/// class and its asymmetry; `None` when nothing differs.
///
/// The first present component decides:
///
/// 1. a discarded fault;
/// 2. an effect difference, staged before committed;
/// 3. an outcome difference;
/// 4. the lowest state component in observation order.
fn metamorphic_divergence(
    differences: &BTreeSet<PpuObservationComponent>,
) -> Option<(
    PpuObservationComponent,
    DivergenceClass,
    CrossReferenceAsymmetry,
)> {
    let component = [
        PpuObservationComponent::FaultDiscard,
        PpuObservationComponent::StagedEffects,
        PpuObservationComponent::CommittedEffects,
        PpuObservationComponent::Outcome,
    ]
    .into_iter()
    .find(|component| differences.contains(component))
    .or_else(|| differences.iter().next().copied())?;
    let (divergence, asymmetry) = match component {
        PpuObservationComponent::FaultDiscard => {
            (DivergenceClass::Outcome, CrossReferenceAsymmetry::Fault)
        }
        PpuObservationComponent::StagedEffects | PpuObservationComponent::CommittedEffects => {
            (DivergenceClass::Effect, CrossReferenceAsymmetry::Effect)
        }
        PpuObservationComponent::Outcome => {
            (DivergenceClass::Outcome, CrossReferenceAsymmetry::Outcome)
        }
        PpuObservationComponent::State
        | PpuObservationComponent::Memory
        | PpuObservationComponent::Reservations
        | PpuObservationComponent::StoreBuffer => (
            DivergenceClass::ArchitecturalState,
            CrossReferenceAsymmetry::State,
        ),
    };
    Some((component, divergence, asymmetry))
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
            component: None,
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

#[cfg(test)]
#[path = "tests/invalid_form_tests.rs"]
mod invalid_form_tests;

#[cfg(test)]
#[path = "tests/metamorphic_component_tests.rs"]
mod metamorphic_component_tests;

#[cfg(test)]
#[path = "tests/reservation_ea_tests.rs"]
mod reservation_ea_tests;
