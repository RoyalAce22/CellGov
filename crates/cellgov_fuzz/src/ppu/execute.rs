//! Single-step and bounded sequence execution of decoded PPU instructions,
//! the observation each run captures, and the asymmetry between a run and
//! its replay.

use std::cell::Cell;

use cellgov_effects::Effect;
use cellgov_event::UnitId;
use cellgov_mem::RegionView;
use cellgov_ppu::exec::{execute, ExecuteVerdict};
use cellgov_ppu::instruction::fuzz::{PpuMetamorphicRelation, PpuOutcomeClass};
use cellgov_ppu::instruction::PpuInstruction;
use cellgov_ppu::observation::{
    finish_observation, PpuArchitecturalState, PpuObservation, PpuObservationCheck,
    PpuObservationComponent, PpuObservationError, PpuObservationInput, PpuObservedOutcome,
};
use cellgov_ppu::state::PpuState;
use cellgov_ppu::store_buffer::StoreBuffer;

use super::generate::DATA_REGION_BASE;
use crate::case::CaseAssessment;
use crate::report::{InstructionIdentity, OutcomeIdentity};
use crate::retention::{CrossReferenceAsymmetry, SemanticObservation, StateTransitionClass};
use crate::seeded;

const UNIT: UnitId = UnitId::new(0);

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ObservedStep {
    pub(super) verdict: ExecuteVerdict,
    pub(super) observation: PpuObservation,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ObservedSequence {
    pub(super) observation: PpuObservation,
    pub(super) terminal_verdict: Option<ExecuteVerdict>,
    pub(super) decode_refusal: Option<(u64, u32)>,
    pub(super) deterministic: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ObservedSequenceRun {
    pub(super) observed: ObservedSequence,
    pub(super) decoded: u64,
    pub(super) decoded_kinds: Vec<InstructionIdentity>,
    pub(super) executed: u64,
    pub(super) executed_kinds: Vec<InstructionIdentity>,
}

#[derive(Clone, Copy)]
pub(super) enum PpuTerminalObservation<'a> {
    Execution(Option<&'a ExecuteVerdict>),
    DecodeRefusal(Option<&'a ExecuteVerdict>),
    CommitRefusal(Option<&'a ExecuteVerdict>),
}

impl<'a> PpuTerminalObservation<'a> {
    pub(super) fn from_step(observed: &'a ObservedStep) -> Self {
        if observed.observation.commit_error.is_some() {
            Self::CommitRefusal(Some(&observed.verdict))
        } else {
            Self::Execution(Some(&observed.verdict))
        }
    }

    pub(super) fn from_sequence(observed: &'a ObservedSequence) -> Self {
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

pub(super) fn run_once(
    instruction: &PpuInstruction,
    initial: &PpuState,
    memory: &[u8],
) -> Result<ObservedStep, PpuObservationError> {
    seeded::executor_boundary();
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
    let mut observation = finish_observation(input)?;
    seeded::ppu_observed(&mut observation);
    Ok(observation)
}

pub(super) fn ppu_observation(
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

pub(super) fn ppu_step_changed(initial: &PpuState, observed: &ObservedStep) -> bool {
    observed.observation.state != PpuArchitecturalState::capture(initial)
}

pub(super) fn ppu_sequence_changed(initial: &PpuState, observed: &ObservedSequence) -> bool {
    observed.observation.state != PpuArchitecturalState::capture(initial)
}

// [Wang2024 p:340:17 s:3.8] A program whose two runs can legitimately differ is unfit for differential comparison, so the engine records replay disagreement as its own finding.
pub(super) fn ppu_step_replay_asymmetry(
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

pub(super) fn ppu_sequence_replay_asymmetry(
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

// [Feng2026 p:32 s:4.3.2] The comparison reduces each run's termination to a normalized signature before it compares the pair.
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

// [Jiang2022 p:7 s:4.2] Most device and emulator inconsistencies differ in the signal raised and few in register or memory values alone, so a fault on either side outranks a state or outcome difference.
pub(super) fn ppu_outcome_asymmetry(outcome: PpuOutcomeClass) -> CrossReferenceAsymmetry {
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

#[cfg(test)]
fn run_sequence(
    words: &[u32],
    initial: &PpuState,
    memory: &[u8],
) -> Result<ObservedSequenceRun, PpuObservationError> {
    run_sequence_tracked(words, initial, memory, &Cell::new(None))
}

/// Runs a sequence and keeps `executing` at the instruction the executor is
/// inside. A panic that unwinds out of the run then still names the
/// instruction that raised it.
pub(super) fn run_sequence_tracked(
    words: &[u32],
    initial: &PpuState,
    memory: &[u8],
    executing: &Cell<Option<InstructionIdentity>>,
) -> Result<ObservedSequenceRun, PpuObservationError> {
    seeded::executor_boundary();
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
        executing.set(None);
        let Ok(instruction) = seeded::ppu_decode(raw) else {
            decode_refusal = Some((state.pc, raw));
            break;
        };
        let descriptor = instruction.fuzz_descriptor(raw);
        deterministic &= requests_replay(descriptor.relations);
        decoded += 1;
        let identity = InstructionIdentity::Ppu(descriptor.kind);
        decoded_kinds.push(identity);
        executing.set(Some(identity));
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

pub(super) fn requests_replay(relations: &[PpuMetamorphicRelation]) -> bool {
    relations.contains(&PpuMetamorphicRelation::Deterministic)
}

pub(super) fn outcome_identity(outcome: PpuOutcomeClass) -> OutcomeIdentity {
    match outcome {
        PpuOutcomeClass::Continue => OutcomeIdentity::PpuContinue,
        PpuOutcomeClass::Branch => OutcomeIdentity::PpuBranch,
        PpuOutcomeClass::Syscall => OutcomeIdentity::PpuSyscall,
        PpuOutcomeClass::Fault => OutcomeIdentity::PpuFault,
        PpuOutcomeClass::MemoryFault => OutcomeIdentity::PpuMemoryFault,
        PpuOutcomeClass::BufferFull => OutcomeIdentity::PpuBufferFull,
    }
}

#[cfg(test)]
#[path = "tests/execute_tests.rs"]
mod tests;
