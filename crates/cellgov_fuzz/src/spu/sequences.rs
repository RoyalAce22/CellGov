//! The SPU sequence engine: a bounded program per case, replayed from local
//! store to find nondeterministic outcomes and footprint violations.

use std::cell::Cell;
use std::collections::BTreeSet;

use cellgov_effects::Effect;
use cellgov_spu::fuzz::{generation_descriptors, SpuOutcomeClass};
use cellgov_spu::observation::SpuObservationComponent;
use cellgov_spu::state::{SpuObservableSnapshot, SPU_LS_SIZE};

use super::assess::assess_sequence_case;
use super::execute::{
    outcome_effects, outcome_identity, run_generated_sequence, spu_observation,
    spu_sequence_replay_asymmetry, SpuTerminalObservation,
};
use super::generate::{
    case_descriptors, random_state, state_aware_state, structured_sequence, GeneratedSequence,
};
use super::record::{guarded_run, record, record_target_panic};
use crate::boundary::call_target;
use crate::case::CaseEligibility;
use crate::error::{FuzzError, InvariantError};
use crate::report::{
    CheckIdentity, DivergenceClass, FindingKind, FuzzReport, FuzzRun, FuzzTarget,
    SemanticFingerprint,
};
use crate::retention::CrossReferenceAsymmetry;
use crate::rng::{Rng, WordGenerationFailure};
use crate::seeded;
use crate::{FuzzConfig, GenerationStrategy};

/// Replays SPU instruction sequences to find nondeterministic outcomes.
pub fn run_sequences(config: FuzzConfig) -> FuzzRun {
    run_sequences_with(config, None)
}

/// Runs the sequence engine; `words` replaces every case's generated words.
pub(crate) fn run_sequences_with(config: FuzzConfig, words: Option<&[u32]>) -> FuzzRun {
    guarded_run(FuzzTarget::SpuSequence, config, |report| {
        run_sequences_inner(config, report, words)
    })
}

/// Generator words for one sequence case, before any check runs.
pub(crate) fn sequence_case_words(
    config: FuzzConfig,
    case_index: u64,
) -> Result<Vec<u32>, FuzzError> {
    config.validate(Some((SPU_LS_SIZE / 4).min(crate::MAX_SEQUENCE_WORDS)))?;
    let descriptors = case_descriptors(config)?;
    let mut rng = Rng::for_case(config.campaign_version, config.seed, case_index);
    let generated = match config.strategy {
        GenerationStrategy::Structured => call_target(|| {
            structured_sequence(&descriptors, &mut rng, config.sequence_words as usize)
        })
        .map_err(|_| InvariantError::UnexpectedPanic {
            stage: "SPU case generation",
        })??,
        GenerationStrategy::RawWords => rng
            .decoder_accepted_words(config.sequence_words as usize, |raw| {
                call_target(|| seeded::spu_decode(raw)).map(|decoded| decoded.is_ok())
            })
            .map(|words| GeneratedSequence {
                words,
                features: BTreeSet::new(),
                interaction: None,
            })
            .map_err(|failure| match failure {
                WordGenerationFailure::DecoderPanic(_, _) => {
                    FuzzError::from(InvariantError::UnexpectedPanic {
                        stage: "SPU case generation",
                    })
                }
                WordGenerationFailure::Exhausted(error) => error.into(),
            })?,
    };
    Ok(generated.words)
}

fn run_sequences_inner(
    config: FuzzConfig,
    report: &mut FuzzReport,
    override_words: Option<&[u32]>,
) -> Result<(), FuzzError> {
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
                    call_target(|| seeded::spu_decode(raw)).map(|decoded| decoded.is_ok())
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
        // The generator draws first so substituted words keep the case's state.
        let words = override_words.map_or(generated.words, <[u32]>::to_vec);
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
        let program_words = words.len();
        let executing = Cell::new(None);
        let first = match call_target(|| {
            run_generated_sequence(
                &initial,
                config.sequence_words as usize,
                program_words,
                config.strategy,
                &executing,
            )
        }) {
            Ok(first) => first,
            Err(payload) => {
                record_target_panic(
                    report,
                    CheckIdentity::SpuExecutor,
                    executing.get(),
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
            let executing = Cell::new(None);
            let second = match call_target(|| {
                run_generated_sequence(
                    &initial,
                    config.sequence_words as usize,
                    program_words,
                    config.strategy,
                    &executing,
                )
            }) {
                Ok(mut second) => {
                    seeded::spu_replayed(&mut second.0.state);
                    second
                }
                Err(payload) => {
                    record_target_panic(
                        report,
                        CheckIdentity::SpuExecutor,
                        executing.get(),
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

#[cfg(test)]
#[path = "tests/sequences_tests.rs"]
mod tests;
