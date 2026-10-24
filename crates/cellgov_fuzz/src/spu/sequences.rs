//! The SPU sequence engine: a bounded program per case, replayed from local
//! store to find nondeterministic outcomes and footprint violations.

use std::cell::Cell;
use std::collections::BTreeSet;

use cellgov_effects::Effect;
use cellgov_spu::fuzz::{SpuGenerationDescriptor, SpuOutcomeClass};
use cellgov_spu::state::{SpuState, SPU_LS_SIZE};

use super::assess::assess_sequence_case;
use super::execute::{
    outcome_effects, outcome_identity, run_generated_sequence_in, spu_observation,
    spu_sequence_replay_asymmetry, ObservedSequenceRun, SequenceWorld, SpuTerminalObservation,
};
use super::generate::{
    case_descriptors, random_state_in, sequence_world, state_aware_state_in, structured_sequence,
    GeneratedSequence,
};
use super::local_stores::LocalStores;
use super::record::{
    footprint_fingerprint, guarded_run, load_descriptors, record, record_target_panic,
};
use super::sequence_relations::{replay_stored, run_relation_check};
use crate::boundary::call_target;
use crate::case::{CaseAssessment, CaseEligibility};
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
    let generated =
        generate_sequence(config, &descriptors, &mut rng).map_err(|failure| match failure {
            WordGenerationFailure::DecoderPanic(_, _) => {
                FuzzError::from(InvariantError::UnexpectedPanic {
                    stage: "SPU case generation",
                })
            }
            WordGenerationFailure::Exhausted(error) => error.into(),
        })?;
    Ok(generated.words)
}

/// The case's generated words: a structured program from `descriptors`,
/// or decoder-accepted raw words. A generator panic is a decoder panic
/// at word 0.
fn generate_sequence(
    config: FuzzConfig,
    descriptors: &[SpuGenerationDescriptor],
    rng: &mut Rng,
) -> Result<GeneratedSequence, WordGenerationFailure> {
    match config.strategy {
        GenerationStrategy::Structured => {
            match call_target(|| {
                structured_sequence(descriptors, rng, config.sequence_words as usize)
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
    }
}

fn run_sequences_inner(
    config: FuzzConfig,
    report: &mut FuzzReport,
    override_words: Option<&[u32]>,
) -> Result<(), FuzzError> {
    config.validate(Some((SPU_LS_SIZE / 4).min(crate::MAX_SEQUENCE_WORDS)))?;
    let iterations = config.case_indices()?;
    let Some(descriptors) = load_descriptors(config.strategy, iterations.clone().next(), report)?
    else {
        return Ok(());
    };
    let pool = LocalStores::default();
    for iteration in iterations {
        // The run that holds case 0 replays the stored counterexamples
        // before it samples a state; a reduction candidate does not.
        if iteration == 0 && override_words.is_none() {
            replay_stored(report)?;
        }
        report.considered()?;
        let mut rng = Rng::for_case(config.campaign_version, config.seed, iteration);
        let generated = match generate_sequence(config, &descriptors, &mut rng) {
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
        let (initial, state_features) = match config.strategy {
            GenerationStrategy::Structured => state_aware_state_in(&mut rng, None, pool.fresh())?,
            GenerationStrategy::RawWords => {
                (random_state_in(&mut rng, pool.fresh())?, BTreeSet::new())
            }
        };
        let mut initial = pool.hold(initial);
        if let Some((interaction, data_base)) = generated.interaction {
            interaction.prepare_state(&mut initial, data_base);
        }
        let world = sequence_world(
            &mut rng,
            generated.interaction.map(|(interaction, _)| interaction),
            config.sequence_words as usize,
        )?;
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
            run_generated_sequence_in(
                &initial,
                &world,
                config.sequence_words as usize,
                program_words,
                config.strategy,
                &executing,
                &pool,
            )
        }) {
            Ok(first) => first?,
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
        report.reached_many(first.decoded, first.decoded_kinds.iter().copied())?;
        let mut features = generated.features;
        features.extend(state_features);
        let assessment = assess_sequence_case(
            config.strategy,
            first.decoded,
            first.observed.has_undefined_operands,
            first.observed.has_unmodeled_execution,
            features,
        );
        report.assessed(&assessment)?;
        report.executed(first.decoded)?;
        report.reached_facilities(&first.observed.coverage);
        // A faulting run discards every effect it committed, so the
        // campaign counts only the effects its last step emitted.
        let emitted = if first.observed.effects.is_empty() {
            first
                .observed
                .terminal_outcome
                .as_ref()
                .map_or(&[][..], outcome_effects)
        } else {
            first.observed.effects.as_slice()
        };
        report.observed_effects(emitted.iter().map(Effect::kind))?;
        // The relation check is independent of the generated program, so
        // every case runs it and every row gets its turn.
        let relation_asymmetry = run_relation_check(report, &mut rng, iteration, &pool)?;
        let case = SequenceCase {
            config,
            iteration,
            words: &words,
            program_words,
            initial: &initial,
            world: &world,
            first: &first,
            assessment: &assessment,
            pool: &pool,
        };
        if assessment.eligibility != CaseEligibility::Eligible {
            case.observe(report, CrossReferenceAsymmetry::None)?;
            continue;
        }
        let Some(mut asymmetry) = case.check_replay(report)? else {
            case.observe(report, CrossReferenceAsymmetry::TargetPanic)?;
            continue;
        };
        asymmetry = asymmetry.max(case.check_program_counter(report)?);
        asymmetry = asymmetry.max(case.check_footprint(report)?);
        asymmetry = asymmetry.max(relation_asymmetry);
        case.observe(report, asymmetry)?;
    }
    Ok(())
}

/// One generated sequence case after its first run, and the checks an
/// eligible case enters. Each check records its findings and returns the
/// asymmetry the case's observation takes.
struct SequenceCase<'a> {
    config: FuzzConfig,
    iteration: u64,
    words: &'a [u32],
    program_words: usize,
    initial: &'a SpuState,
    world: &'a SequenceWorld,
    first: &'a ObservedSequenceRun,
    assessment: &'a CaseAssessment,
    pool: &'a LocalStores,
}

impl SequenceCase<'_> {
    fn observe(
        &self,
        report: &mut FuzzReport,
        asymmetry: CrossReferenceAsymmetry,
    ) -> Result<(), InvariantError> {
        let first = self.first;
        report.observe_case(
            self.iteration,
            spu_observation(
                first.decoded_kinds.iter().copied(),
                self.assessment,
                SpuTerminalObservation::from_sequence(&first.observed),
                &first.observed.state,
                self.initial,
                first.decoded,
                asymmetry,
            ),
        )?;
        Ok(())
    }

    fn fingerprint(
        &self,
        check: CheckIdentity,
        divergence: DivergenceClass,
    ) -> SemanticFingerprint {
        SemanticFingerprint {
            target: FuzzTarget::SpuSequence,
            instruction_kind: None,
            check,
            divergence,
            outcome: None,
            effect: None,
            component: None,
        }
    }

    /// The replay's asymmetry, `None` asymmetry for a sequence that asks
    /// for no replay; `Ok(None)` when the replay panicked, recorded here.
    fn check_replay(
        &self,
        report: &mut FuzzReport,
    ) -> Result<Option<CrossReferenceAsymmetry>, InvariantError> {
        if !self.first.observed.deterministic {
            return Ok(Some(CrossReferenceAsymmetry::None));
        }
        let executing = Cell::new(None);
        let world = self.world;
        let late = seeded::spu_replay_actions(&world.actions).map(|actions| SequenceWorld {
            memory: world.memory.clone(),
            actions,
        });
        let replay_world = late.as_ref().unwrap_or(world);
        let second = match call_target(|| {
            run_generated_sequence_in(
                self.initial,
                replay_world,
                self.config.sequence_words as usize,
                self.program_words,
                self.config.strategy,
                &executing,
                self.pool,
            )
        }) {
            Ok(second) => {
                let mut second = second?;
                seeded::spu_replayed(&mut second.observed.state);
                second
            }
            Err(payload) => {
                record_target_panic(
                    report,
                    CheckIdentity::SpuExecutor,
                    executing.get(),
                    self.words.to_vec(),
                    self.iteration,
                    payload,
                )?;
                return Ok(None);
            }
        };
        let replay_asymmetry = spu_sequence_replay_asymmetry(self.first, &second);
        if replay_asymmetry != CrossReferenceAsymmetry::None {
            record(
                report,
                FindingKind::Nondeterministic,
                self.fingerprint(
                    CheckIdentity::DeterministicReplay,
                    DivergenceClass::ArchitecturalState,
                ),
                self.words.to_vec(),
                self.iteration,
            )?;
        }
        Ok(Some(replay_asymmetry))
    }

    /// A finding when the run ends outside local store or off a word
    /// boundary.
    fn check_program_counter(
        &self,
        report: &mut FuzzReport,
    ) -> Result<CrossReferenceAsymmetry, InvariantError> {
        let pc = self.first.observed.state.pc;
        if (pc as usize) < SPU_LS_SIZE && pc & 3 == 0 {
            return Ok(CrossReferenceAsymmetry::None);
        }
        record(
            report,
            FindingKind::InvalidProgramCounter,
            self.fingerprint(CheckIdentity::ProgramCounter, DivergenceClass::ControlFlow),
            self.words.to_vec(),
            self.iteration,
        )?;
        Ok(CrossReferenceAsymmetry::Fault)
    }

    /// One finding per component the run wrote outside its allowed
    /// footprint.
    fn check_footprint(
        &self,
        report: &mut FuzzReport,
    ) -> Result<CrossReferenceAsymmetry, InvariantError> {
        let observed = &self.first.observed;
        let mut asymmetry = CrossReferenceAsymmetry::None;
        for component in &observed.footprint_violations {
            asymmetry = CrossReferenceAsymmetry::State;
            record(
                report,
                FindingKind::IllegalFootprint,
                footprint_fingerprint(
                    FuzzTarget::SpuSequence,
                    None,
                    *component,
                    observed
                        .terminal_outcome
                        .as_ref()
                        .map(|outcome| outcome_identity(SpuOutcomeClass::from_outcome(outcome))),
                ),
                self.words.to_vec(),
                self.iteration,
            )?;
        }
        Ok(asymmetry)
    }
}

#[cfg(test)]
#[path = "tests/sequences_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/fall_through_wrap_tests.rs"]
mod fall_through_wrap_tests;

#[cfg(test)]
#[path = "tests/facility_tests.rs"]
mod facility_tests;
