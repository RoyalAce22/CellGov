//! Single-step and bounded sequence execution of decoded SPU instructions,
//! the observation each run captures, and the asymmetry between a run and
//! its replay.

use std::cell::Cell;
use std::collections::BTreeSet;

use cellgov_effects::Effect;
use cellgov_event::UnitId;
use cellgov_exec::YieldReason;
use cellgov_ps3_abi::hw::spu;
use cellgov_spu::exec::{execute, SpuStepOutcome};
use cellgov_spu::fuzz::{
    encoding_execution_is_supported, encoding_has_undefined_operands, SpuMetamorphicRelation,
    SpuOutcomeClass,
};
use cellgov_spu::instruction::SpuInstruction;
use cellgov_spu::observation::{SpuAllowedFootprint, SpuObservation, SpuObservationComponent};
use cellgov_spu::state::{SpuObservableSnapshot, SpuState};
use cellgov_time::GuestTicks;

use super::local_stores::{LocalStores, Pooled};
use crate::case::CaseAssessment;
use crate::error::InvariantError;
use crate::report::{InstructionIdentity, OutcomeIdentity, SpuFacilityCoverage};
use crate::retention::{CrossReferenceAsymmetry, SemanticObservation, StateTransitionClass};
use crate::seeded;
use crate::spu_reference::world::{World, OWN};
use crate::spu_reference::{
    SpuReferenceError, SpuReferenceInvalidCommand, SpuReferencePpuAction, SpuReferencePpuResult,
};
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
    /// Each main-storage region's bytes at the end.
    pub(super) memory: Vec<(u64, Vec<u8>)>,
    /// Every effect the world committed, in emission order.
    pub(super) effects: Vec<Effect>,
    /// One result per problem-state operation the world carried out.
    pub(super) ppu_results: Vec<SpuReferencePpuResult>,
    /// Commands the MFC refused when its queue reached them.
    pub(super) mfc_exceptions: Vec<SpuReferenceInvalidCommand>,
    /// Whether the run ended parked on a blocked channel.
    pub(super) stalled: bool,
    /// How many times a blocked access parked and ran again.
    pub(super) parks: u32,
    /// The channels, MFC commands and event sources the run reached.
    pub(super) coverage: SpuFacilityCoverage,
}

/// The outside world a generated sequence runs against.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct SequenceWorld {
    /// Main storage, one region per `(base, bytes)` run.
    pub(super) memory: Vec<(u64, Vec<u8>)>,
    /// The problem-state operations, in order, each with the count of
    /// decoded instructions it lands after.
    pub(super) actions: Vec<(u64, SpuReferencePpuAction)>,
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

#[cfg(test)]
pub(super) fn run_once(
    instruction: &cellgov_spu::instruction::SpuInstruction,
    initial: &SpuState,
) -> ObservedStep {
    observe_step(instruction, initial.clone())
}

/// Executes `instruction` once from a copy of `initial` in a local store
/// from `pool`.
pub(super) fn run_once_in<'p>(
    instruction: &cellgov_spu::instruction::SpuInstruction,
    initial: &SpuState,
    pool: &'p LocalStores,
) -> Pooled<'p, ObservedStep> {
    pool.hold(observe_step(instruction, pool.copy(initial)))
}

/// Executes `instruction` once from `state`, a copy of the case's initial
/// state.
fn observe_step(
    instruction: &cellgov_spu::instruction::SpuInstruction,
    mut state: SpuState,
) -> ObservedStep {
    seeded::executor_boundary();
    let outcome = execute(instruction, &mut state, UNIT);
    seeded::spu_observed(
        instruction,
        SpuOutcomeClass::from_outcome(&outcome),
        &mut state,
    );
    ObservedStep {
        outcome,
        state: SpuObservableSnapshot::capture_owned(state),
    }
}

pub(super) fn spu_observation(
    kinds: impl IntoIterator<Item = InstructionIdentity>,
    assessment: &CaseAssessment,
    terminal: SpuTerminalObservation<'_>,
    state: &SpuObservableSnapshot,
    initial_state: &SpuState,
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
    let state_changed = !state.matches(initial_state);
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

/// [Wang2024 p:340:17 s:3.8] A program whose two runs can legitimately differ is unfit for differential comparison, so the engine records replay disagreement as its own finding.
pub(super) fn spu_step_replay_asymmetry(
    first: &ObservedStep,
    second: &ObservedStep,
) -> CrossReferenceAsymmetry {
    let differences =
        SpuObservation::compare_parts(&first.state, &first.outcome, &second.state, &second.outcome)
            .differences;
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
        || first.0.memory != second.0.memory
        || first.0.ppu_results != second.0.ppu_results
        || first.0.mfc_exceptions != second.0.mfc_exceptions
        || first.0.stalled != second.0.stalled
        || first.0.parks != second.0.parks
        || first.0.coverage != second.0.coverage
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
        optional_outcome_effects(first_outcome) != optional_outcome_effects(second_outcome)
            || first.0.effects != second.0.effects,
    )
}

/// [Feng2026 p:32 s:4.3.2] The comparison reduces each run's termination to a normalized signature before it compares the pair.
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

/// [Jiang2022 p:7 s:4.2] Most device and emulator inconsistencies differ in the signal raised and few in register or memory values alone, so a fault on either side outranks a state or outcome difference.
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
    run_sequence_with_limit(
        initial,
        &SequenceWorld::default(),
        budget,
        None,
        &Cell::new(None),
        &LocalStores::default(),
    )
    .expect("an empty world builds")
}

#[cfg(test)]
pub(super) fn run_generated_sequence(
    initial: &SpuState,
    world: &SequenceWorld,
    budget: usize,
    program_words: usize,
    strategy: GenerationStrategy,
    executing: &Cell<Option<InstructionIdentity>>,
) -> Result<(ObservedSequence, u64, Vec<InstructionIdentity>), InvariantError> {
    run_sequence_with_limit(
        initial,
        world,
        budget,
        program_bound(strategy, program_words),
        executing,
        &LocalStores::default(),
    )
}

/// Runs a generated sequence from `initial` against `world`, with its local
/// stores from `pool`; a structured run stops at its program's end.
pub(super) fn run_generated_sequence_in<'p>(
    initial: &SpuState,
    world: &SequenceWorld,
    budget: usize,
    program_words: usize,
    strategy: GenerationStrategy,
    executing: &Cell<Option<InstructionIdentity>>,
    pool: &'p LocalStores,
) -> Result<Pooled<'p, (ObservedSequence, u64, Vec<InstructionIdentity>)>, InvariantError> {
    run_sequence_with_limit(
        initial,
        world,
        budget,
        program_bound(strategy, program_words),
        executing,
        pool,
    )
    .map(|run| pool.hold(run))
}

/// The word count a structured run stops at; a raw-word run has none.
fn program_bound(strategy: GenerationStrategy, program_words: usize) -> Option<usize> {
    match strategy {
        GenerationStrategy::Structured => Some(program_words),
        GenerationStrategy::RawWords => None,
    }
}

/// Names the part of the world that refused, as an invariant error.
fn world_failure(error: SpuReferenceError) -> InvariantError {
    InvariantError::SequenceWorld {
        what: match error {
            SpuReferenceError::World { what } => what,
            _ => "its state",
        },
    }
}

/// Adds the channel `instruction` names, and the MFC command a write of
/// `MFC_Cmd` issued, to `coverage`.
fn record_coverage(
    coverage: &mut SpuFacilityCoverage,
    instruction: &SpuInstruction,
    before: &SpuState,
    outcome: &SpuStepOutcome,
) {
    let (SpuInstruction::Rdch { channel, .. }
    | SpuInstruction::Wrch { channel, .. }
    | SpuInstruction::Rchcnt { channel, .. }) = *instruction
    else {
        return;
    };
    coverage.channels.insert(channel);
    let stalled = matches!(
        outcome,
        SpuStepOutcome::Yield {
            reason: YieldReason::ChannelStall,
            ..
        }
    );
    if let SpuInstruction::Wrch {
        channel: spu::MFC_CMD,
        rt,
    } = *instruction
    {
        if !stalled {
            // [CBEA p:114 s:9.1.2 MFC Class ID Channel] the command parameter is the low 16 bits of the word written to MFC_Cmd, and the class IDs the high 16.
            // [CBE-Handbook p:457 s:17.9.6 MFC Class ID and MFC Command Opcode Channel] bits 24:31 hold the opcode, and bit 16 set marks a reserved one, so the whole halfword is kept.
            coverage.mfc_opcodes.insert(before.reg_word(rt) as u16);
        }
    }
}

/// Runs a sequence against `script`'s world and keeps `executing` at the
/// instruction the executor is inside. A panic that unwinds out of the run
/// then still names the instruction that raised it.
///
/// Each instruction runs as one step of the execution unit runs it:
///
/// 1. The world refreshes the SPU's view.
/// 2. The instruction executes.
/// 3. The world commits and completes the transfers and stores a yield
///    emits, and lands an atomic read in local store.
///
/// A blocked channel access parks. It runs again when the next scripted
/// operation lands. If no operation is left, the run ends there.
fn run_sequence_with_limit(
    initial: &SpuState,
    script: &SequenceWorld,
    budget: usize,
    program_words: Option<usize>,
    executing: &Cell<Option<InstructionIdentity>>,
    pool: &LocalStores,
) -> Result<(ObservedSequence, u64, Vec<InstructionIdentity>), InvariantError> {
    seeded::executor_boundary();
    let mut world = World::with_local_store(initial, pool.take(), &script.memory, None)
        .map_err(world_failure)?;
    let mut actions = script.actions.iter().peekable();
    let mut decoded = 0u64;
    let mut kinds = Vec::new();
    let mut terminal_outcome = None;
    let mut decode_refusal = None;
    let mut deterministic = true;
    let mut has_undefined_operands = false;
    let mut has_unmodeled_execution = false;
    let mut footprint_violations = BTreeSet::new();
    let mut coverage = SpuFacilityCoverage::default();
    let mut pending_seen = 0u32;
    let mut effects = Vec::new();
    let mut stalled = false;
    let mut faulted = false;
    // A parked access runs again without counting as a new instruction.
    let mut parked = false;
    let mut parks = 0u32;
    let mut retired = 0usize;
    // Each step's pre-state, copied into one buffer the run reuses from
    // its first step on.
    let mut pre_state: Option<SpuState> = None;
    while retired < budget {
        while let Some((_, action)) = actions.next_if(|(after, _)| *after <= decoded) {
            world.apply(*action);
        }
        world.enter();
        // [SPU-ISA p:251 s:12.1] an interrupt branches to address 0, so a parked access gives way to a new instruction.
        if cellgov_spu::step::take_pending_interrupt(world.unit.state_mut()) {
            parked = false;
        }
        let state = world.unit.state();
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
        let identity = InstructionIdentity::Spu(descriptor.kind);
        if !parked {
            decoded += 1;
            kinds.push(identity);
        }
        executing.set(Some(identity));
        let first_step = pre_state.is_none();
        let before = pre_state.get_or_insert_with(|| pool.copy(state));
        if !first_step {
            before.clone_from(state);
        }
        let state = world.unit.state_mut();
        let outcome = cellgov_spu::step::execute_instruction(&instruction, state, OWN);
        seeded::spu_observed(&instruction, SpuOutcomeClass::from_outcome(&outcome), state);
        footprint_violations.extend(
            SpuAllowedFootprint::for_instruction(&instruction)
                .violations_after(before, state, &outcome),
        );
        record_coverage(&mut coverage, &instruction, before, &outcome);
        pending_seen |= state.channels.pending_events;
        terminal_outcome = Some(outcome.clone());
        parked = false;
        match outcome {
            SpuStepOutcome::Continue => state.advance_pc(),
            SpuStepOutcome::Branch => {}
            SpuStepOutcome::Yield {
                effects: emitted,
                reason,
            } => {
                let blocked = reason == YieldReason::ChannelStall;
                // A blocked access did not retire, so the program counter
                // stays on it.
                if !blocked {
                    state.advance_pc();
                }
                world.settle(&emitted).map_err(world_failure)?;
                effects.extend(emitted);
                if blocked {
                    // [CBE-Handbook p:447 s:17.1.6] a blocked access stalls until the channel changes or the SPU is interrupted.
                    let Some((_, action)) = actions.next() else {
                        stalled = true;
                        break;
                    };
                    world.apply(*action);
                    parked = true;
                    parks += 1;
                    continue;
                }
            }
            SpuStepOutcome::MemoryRead {
                ea,
                lsa,
                size,
                acquire_line,
            } => {
                let mut emitted = Vec::new();
                world.complete_read(ea, lsa, size, acquire_line, &mut emitted);
                world.unit.state_mut().advance_pc();
                world.settle(&emitted).map_err(world_failure)?;
                effects.extend(emitted);
            }
            // [SPU-ISA p:238 s:10. Control Instructions] a stop leaves PC at the next word, masked by LSLR.
            SpuStepOutcome::Stop { kind, signal } => {
                state.record_stop(kind, signal);
                break;
            }
            SpuStepOutcome::Fault(_) => {
                faulted = true;
                break;
            }
        }
        // A store the world could not commit refuses the batch, as a fault does.
        if world.refused {
            faulted = true;
            break;
        }
        // One instruction per nanosecond. The conversion reads the
        // running total, so the next instruction gets each dropped
        // partial tick.
        let before = cellgov_time::nsec_to_ticks(retired as u64);
        retired += 1;
        world
            .advance(GuestTicks::new(
                cellgov_time::nsec_to_ticks(retired as u64) - before,
            ))
            .map_err(world_failure)?;
    }
    if let Some(before) = pre_state {
        pool.give(before.ls);
    }
    let ppu_results = std::mem::take(&mut world.ppu_results);
    let mfc_exceptions = std::mem::take(&mut world.exceptions);
    let (mut state, memory) = if faulted {
        // The runtime fault-discard rule hides state from a faulting batch.
        // Main storage is then the script's runs, which the world started
        // from.
        effects.clear();
        pool.give(world.into_state().ls);
        (pool.copy(initial), script.memory.clone())
    } else {
        let memory = world.memory_image().map_err(world_failure)?;
        (world.into_state(), memory)
    };
    coverage.event_sources = (0..32)
        .map(|bit| 1u32 << bit)
        .filter(|mask| pending_seen & spu::event::ALL & mask != 0)
        .collect();
    seeded::spu_program_counter(&mut state.pc);
    Ok((
        ObservedSequence {
            state: SpuObservableSnapshot::capture_owned(state),
            // Sequence replay retains the terminal outcome because the descriptor observes its effects.
            terminal_outcome,
            decode_refusal,
            deterministic,
            has_undefined_operands,
            has_unmodeled_execution,
            footprint_violations,
            memory,
            effects,
            ppu_results,
            mfc_exceptions,
            stalled,
            parks,
            coverage,
        },
        decoded,
        kinds,
    ))
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
        SpuOutcomeClass::Stop => OutcomeIdentity::SpuStop,
    }
}

pub(super) fn outcome_effects(outcome: &SpuStepOutcome) -> &[Effect] {
    match outcome {
        SpuStepOutcome::Yield { effects, .. } => effects,
        SpuStepOutcome::Continue
        | SpuStepOutcome::Branch
        | SpuStepOutcome::MemoryRead { .. }
        | SpuStepOutcome::Fault(_)
        | SpuStepOutcome::Stop { .. } => &[],
    }
}

#[cfg(test)]
#[path = "tests/execute_tests.rs"]
mod tests;
