//! Single-step and bounded sequence execution of decoded SPU instructions,
//! the observation each run captures, and the asymmetry between a run and
//! its replay.

use std::cell::Cell;
use std::collections::BTreeSet;

use cellgov_effects::Effect;
use cellgov_event::UnitId;
use cellgov_spu::exec::{execute, SpuStepOutcome};
use cellgov_spu::fuzz::{
    encoding_execution_is_supported, encoding_has_undefined_operands, SpuMetamorphicRelation,
    SpuOutcomeClass,
};
use cellgov_spu::observation::{SpuAllowedFootprint, SpuObservation, SpuObservationComponent};
use cellgov_spu::state::{SpuObservableSnapshot, SpuState};

use crate::case::CaseAssessment;
use crate::report::{InstructionIdentity, OutcomeIdentity};
use crate::retention::{CrossReferenceAsymmetry, SemanticObservation, StateTransitionClass};
use crate::seeded;
use crate::GenerationStrategy;

const UNIT: UnitId = UnitId::new(0);

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ObservedStep {
    pub(super) outcome: SpuStepOutcome,
    pub(super) state: SpuObservableSnapshot,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ObservedSequence {
    pub(super) state: SpuObservableSnapshot,
    pub(super) terminal_outcome: Option<SpuStepOutcome>,
    pub(super) decode_refusal: Option<(u32, u32)>,
    pub(super) deterministic: bool,
    pub(super) has_undefined_operands: bool,
    pub(super) has_unmodeled_execution: bool,
    pub(super) footprint_violations: BTreeSet<SpuObservationComponent>,
}

#[derive(Clone, Copy)]
pub(super) enum SpuTerminalObservation<'a> {
    Execution(Option<&'a SpuStepOutcome>),
    DecodeRefusal(Option<&'a SpuStepOutcome>),
}

impl<'a> SpuTerminalObservation<'a> {
    pub(super) fn from_sequence(observed: &'a ObservedSequence) -> Self {
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

pub(super) fn run_once(
    instruction: &cellgov_spu::instruction::SpuInstruction,
    initial: &SpuState,
) -> ObservedStep {
    seeded::executor_boundary();
    let mut state = initial.clone();
    let outcome = execute(instruction, &mut state, UNIT);
    seeded::spu_observed(
        instruction,
        SpuOutcomeClass::from_outcome(&outcome),
        &mut state.regs,
    );
    ObservedStep {
        outcome,
        state: SpuObservableSnapshot::capture(&state),
    }
}

pub(super) fn spu_observation(
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

// [Wang2024 p:340:17 s:3.8] A program whose two runs can legitimately differ is unfit for differential comparison, so the engine records replay disagreement as its own finding.
pub(super) fn spu_step_replay_asymmetry(
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

pub(super) fn spu_sequence_replay_asymmetry(
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

// [Feng2026 p:32 s:4.3.2] The comparison reduces each run's termination to a normalized signature before it compares the pair.
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

pub(super) fn spu_outcome_class_asymmetry(outcome: SpuOutcomeClass) -> CrossReferenceAsymmetry {
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

#[cfg(test)]
fn run_sequence(
    initial: &SpuState,
    budget: usize,
) -> (ObservedSequence, u64, Vec<InstructionIdentity>) {
    run_sequence_with_limit(initial, budget, None, &Cell::new(None))
}

pub(super) fn run_generated_sequence(
    initial: &SpuState,
    budget: usize,
    program_words: usize,
    strategy: GenerationStrategy,
    executing: &Cell<Option<InstructionIdentity>>,
) -> (ObservedSequence, u64, Vec<InstructionIdentity>) {
    match strategy {
        GenerationStrategy::Structured => {
            run_sequence_with_limit(initial, budget, Some(program_words), executing)
        }
        GenerationStrategy::RawWords => run_sequence_with_limit(initial, budget, None, executing),
    }
}

/// Runs a sequence and keeps `executing` at the instruction the executor is
/// inside. A panic that unwinds out of the run then still names the
/// instruction that raised it.
fn run_sequence_with_limit(
    initial: &SpuState,
    budget: usize,
    program_words: Option<usize>,
    executing: &Cell<Option<InstructionIdentity>>,
) -> (ObservedSequence, u64, Vec<InstructionIdentity>) {
    seeded::executor_boundary();
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
        executing.set(None);
        let Ok(instruction) = seeded::spu_decode(raw) else {
            decode_refusal = Some((state.pc, raw));
            break;
        };
        let descriptor = instruction.fuzz_descriptor();
        has_undefined_operands |= encoding_has_undefined_operands(raw);
        has_unmodeled_execution |= !encoding_execution_is_supported(raw);
        deterministic &= requests_replay(descriptor.relations);
        decoded += 1;
        let identity = InstructionIdentity::Spu(descriptor.kind);
        kinds.push(identity);
        executing.set(Some(identity));
        let before = state.clone();
        let outcome = execute(&instruction, &mut state, UNIT);
        seeded::spu_observed(
            &instruction,
            SpuOutcomeClass::from_outcome(&outcome),
            &mut state.regs,
        );
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
    seeded::spu_program_counter(&mut state.pc);
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

pub(super) fn requests_replay(relations: &[SpuMetamorphicRelation]) -> bool {
    relations.contains(&SpuMetamorphicRelation::Deterministic)
}

pub(super) fn outcome_identity(outcome: SpuOutcomeClass) -> OutcomeIdentity {
    match outcome {
        SpuOutcomeClass::Continue => OutcomeIdentity::SpuContinue,
        SpuOutcomeClass::Branch => OutcomeIdentity::SpuBranch,
        SpuOutcomeClass::Yield => OutcomeIdentity::SpuYield,
        SpuOutcomeClass::MemoryRead => OutcomeIdentity::SpuMemoryRead,
        SpuOutcomeClass::Fault => OutcomeIdentity::SpuFault,
    }
}

pub(super) fn outcome_effects(outcome: &SpuStepOutcome) -> &[Effect] {
    match outcome {
        SpuStepOutcome::Yield { effects, .. } => effects,
        SpuStepOutcome::Continue
        | SpuStepOutcome::Branch
        | SpuStepOutcome::MemoryRead { .. }
        | SpuStepOutcome::Fault(_) => &[],
    }
}

#[cfg(test)]
#[path = "tests/execute_tests.rs"]
mod tests;
