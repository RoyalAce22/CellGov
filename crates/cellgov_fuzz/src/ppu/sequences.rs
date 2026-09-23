//! The PPU sequence engine: a bounded program per case, replayed from a
//! generated word list to find nondeterministic outcomes.

use std::cell::Cell;
use std::collections::BTreeSet;

use cellgov_effects::Effect;
use cellgov_ppu::instruction::fuzz::generation_descriptors;

use super::assess::assess_sequence_case;
use super::execute::{
    ppu_observation, ppu_sequence_changed, ppu_sequence_replay_asymmetry, run_sequence_tracked,
    PpuTerminalObservation,
};
use super::generate::{
    case_descriptors, state_aware_state_for_sequence, structured_sequence, GeneratedSequence,
    DATA_LEN,
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

/// Replays PPU instruction sequences to find nondeterministic outcomes.
pub fn run_sequences(config: FuzzConfig) -> FuzzRun {
    run_sequences_with(config, None)
}

/// Runs the sequence engine; `words` replaces every case's generated words.
pub(crate) fn run_sequences_with(config: FuzzConfig, words: Option<&[u32]>) -> FuzzRun {
    guarded_run(FuzzTarget::PpuSequence, config, |report| {
        run_sequences_inner(config, report, words)
    })
}

/// Generator words for one sequence case, before any check runs.
pub(crate) fn sequence_case_words(
    config: FuzzConfig,
    case_index: u64,
) -> Result<Vec<u32>, FuzzError> {
    config.validate(Some(crate::MAX_SEQUENCE_WORDS))?;
    let descriptors = case_descriptors(config)?;
    let mut rng = Rng::for_case(config.campaign_version, config.seed, case_index);
    let generated = match config.strategy {
        GenerationStrategy::Structured => call_target(|| {
            structured_sequence(&descriptors, &mut rng, config.sequence_words as usize)
        })
        .map_err(|_| InvariantError::UnexpectedPanic {
            stage: "PPU case generation",
        })??,
        GenerationStrategy::RawWords => rng
            .decoder_accepted_words(config.sequence_words as usize, |raw| {
                call_target(|| seeded::ppu_decode(raw)).map(|decoded| decoded.is_ok())
            })
            .map(|words| GeneratedSequence {
                words,
                features: BTreeSet::new(),
            })
            .map_err(|failure| match failure {
                WordGenerationFailure::DecoderPanic(_, _) => {
                    FuzzError::from(InvariantError::UnexpectedPanic {
                        stage: "PPU case generation",
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
                    call_target(|| seeded::ppu_decode(raw)).map(|decoded| decoded.is_ok())
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
        // The generator draws first so substituted words keep the case's state.
        let words = override_words.map_or(generated.words, <[u32]>::to_vec);
        let (mut initial, state_features) =
            state_aware_state_for_sequence(config.strategy, &mut rng)?;
        initial.pc = 0;
        let mut memory = vec![0u8; DATA_LEN];
        rng.fill(&mut memory);
        if words.is_empty() {
            return Err(InvariantError::EmptyGeneratedSequence.into());
        }
        let executing = Cell::new(None);
        let first =
            match call_target(|| run_sequence_tracked(&words, &initial, &memory, &executing)) {
                Ok(Ok(first)) => first,
                Ok(Err(error)) => return Err(error.into()),
                Err(payload) => {
                    record_target_panic(
                        report,
                        CheckIdentity::PpuExecutor,
                        executing.get(),
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
            let executing = Cell::new(None);
            let second =
                match call_target(|| run_sequence_tracked(&words, &initial, &memory, &executing)) {
                    Ok(Ok(mut second)) => {
                        seeded::ppu_replayed(&mut second.observed.observation);
                        second
                    }
                    Ok(Err(error)) => return Err(error.into()),
                    Err(payload) => {
                        record_target_panic(
                            report,
                            CheckIdentity::PpuExecutor,
                            executing.get(),
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

#[cfg(test)]
#[path = "tests/sequences_tests.rs"]
mod tests;
