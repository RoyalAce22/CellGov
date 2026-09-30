//! The sequence-relation runner: one catalog row, instantiated on real
//! registers and a start state, runs as sequence A and as its partner, and
//! the runner compares the two final observations.
//!
//! The runner executes the row words directly and not through the seeded
//! hooks the other tiers own, so a seeded replay, footprint or common-mode
//! defect stays a finding of its own check.

use cellgov_event::UnitId;
use cellgov_spu::exec::{execute, SpuStepOutcome};
use cellgov_spu::fuzz::{
    sequence_relations, SpuSequencePartner, SpuSequenceRelation, SpuSequenceRelationId,
    SpuSymbolicWord,
};
use cellgov_spu::observation::{SpuObservation, SpuObservationComponent};
use cellgov_spu::state::{SpuState, SPU_REG_COUNT};

use super::record::record;
use crate::error::{FuzzError, InvariantError};
use crate::report::{
    CheckIdentity, DivergenceClass, FindingKind, FuzzReport, FuzzTarget, SemanticFingerprint,
    SequenceRelationDivergence,
};
use crate::retention::CrossReferenceAsymmetry;
use crate::rng::Rng;

const UNIT: UnitId = UnitId::new(0);

/// `stop 0x3FFE`: the terminator both sides of a relation stop at.
/// [SPU-ISA p:238 s:10 Stop] opcode 0x000 with the signal type in bits 18:31.
const TERMINATOR: u32 = 0x0000_3FFE;

/// Lane values the instantiation draws beside random words: zero, one, the
/// signed extremes, all ones and a halfword boundary.
const BOUNDARY_WORDS: [u32; 6] = [0, 1, 0x7FFF_FFFF, 0x8000_0000, 0xFFFF_FFFF, 0x0000_FFFF];

/// A relation row on real registers and a start state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RelationInstance {
    /// The real register of each symbolic register.
    pub(crate) assignment: Vec<u8>,
    /// The state both sides start from.
    pub(crate) start: SpuState,
}

/// What one instantiated relation showed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RelationVerdict {
    /// The start state lies outside the row's precondition; nothing was
    /// compared.
    Inapplicable,
    /// Both sides left the same observation.
    Match,
    /// The sides differ.
    Diverged(Box<SequenceRelationDivergence>),
}

/// Draws an instantiation of `relation`.
///
/// [Bansal2006 p:396 s:3.1] A row numbers its registers in order of first
/// appearance, so it stands for every assignment of distinct registers;
/// the draw picks one, and a third of the draws alias a later symbolic
/// register onto an earlier one (`ra == rb`, `rt == ra`).
pub(crate) fn instantiate(
    relation: &SpuSequenceRelation,
    rng: &mut Rng,
) -> Result<RelationInstance, FuzzError> {
    let count = relation.register_count();
    let mut assignment: Vec<u8> = Vec::with_capacity(count);
    while assignment.len() < count {
        let register = rng.below(SPU_REG_COUNT as u64)? as u8;
        if !assignment.contains(&register) {
            assignment.push(register);
        }
    }
    if count >= 2 && rng.chance(1, 3)? {
        let later = 1 + rng.below(count as u64 - 1)? as usize;
        let earlier = rng.below(later as u64)? as usize;
        assignment[later] = assignment[earlier];
    }
    let mut start = SpuState::new();
    for register in 0..SPU_REG_COUNT {
        let mut value = [0u8; 16];
        rng.fill(&mut value);
        start.set_reg(register, value);
    }
    for &register in &assignment {
        let mut value = [0u8; 16];
        for lane in value.chunks_exact_mut(4) {
            let word = if rng.chance(1, 2)? {
                BOUNDARY_WORDS[rng.below(BOUNDARY_WORDS.len() as u64)? as usize]
            } else {
                rng.next_u32()
            };
            lane.copy_from_slice(&word.to_be_bytes());
        }
        start.set_reg(usize::from(register), value);
    }
    // Equal lanes on two inputs, so a compare can hold.
    if assignment.len() >= 3 && rng.chance(1, 4)? {
        let value = start.regs[usize::from(assignment[1])];
        start.set_reg(usize::from(assignment[2]), value);
    }
    Ok(RelationInstance { assignment, start })
}

/// Runs both sides of `relation` from `instance` and compares them.
///
/// [Le2014 p:219 s:3.1.1] The partner is equivalent only over the inputs on
/// which both have a defined result. The precondition names that domain, and
/// the runner does not compare a start state outside it.
pub(crate) fn compare_relation(
    relation: &SpuSequenceRelation,
    instance: &RelationInstance,
    case_index: u64,
) -> Result<RelationVerdict, FuzzError> {
    if let Some(precondition) = relation.precondition {
        if !precondition(&instance.start, &instance.assignment) {
            return Ok(RelationVerdict::Inapplicable);
        }
    }
    let words = encode(relation.id, relation.sequence, &instance.assignment)?;
    let original = run_side(relation.id, &words, &instance.start)?;
    let partner = match relation.partner {
        SpuSequencePartner::Guest(partner) => {
            let partner = encode(relation.id, partner, &instance.assignment)?;
            run_side(relation.id, &partner, &instance.start)?
        }
        SpuSequencePartner::Fused(fused) => {
            let mut state = instance.start.clone();
            (fused.apply)(&mut state, &instance.assignment);
            run_side(relation.id, &[], &state)?
        }
    };
    // [Martignoni2009 p:127 s:2.2] The comparison covers the complete state
    // after execution.
    let differences = original.compare(&partner).differences;
    let Some(&first_component) = differences.iter().next() else {
        return Ok(RelationVerdict::Match);
    };
    Ok(RelationVerdict::Diverged(Box::new(
        SequenceRelationDivergence {
            relation: relation.id,
            case_index,
            first_component,
            start_registers: Box::new(*instance.start.regs.as_array()),
            assignment: instance.assignment.clone(),
            bit_distance: bit_distance(&original, &partner),
        },
    )))
}

/// Checks one catalog row, drawn with the case's generator, and records
/// what it shows.
///
/// [Martignoni2009 p:127 s:2.2] A divergence is a finding against the row's
/// own check, with the words of sequence A as its reproducer.
pub(super) fn run_relation_check(
    report: &mut FuzzReport,
    rng: &mut Rng,
    case_index: u64,
) -> Result<CrossReferenceAsymmetry, FuzzError> {
    let relations = sequence_relations();
    if relations.is_empty() {
        return Ok(CrossReferenceAsymmetry::None);
    }
    let relation = &relations[rng.below(relations.len() as u64)? as usize];
    let check = CheckIdentity::SpuSequenceRelation(relation.id);
    let instance = instantiate(relation, rng)?;
    match compare_relation(relation, &instance, case_index)? {
        RelationVerdict::Inapplicable => {
            report.metamorphic_skipped(check)?;
            Ok(CrossReferenceAsymmetry::None)
        }
        RelationVerdict::Match => {
            report.metamorphic_executed(check)?;
            Ok(CrossReferenceAsymmetry::None)
        }
        RelationVerdict::Diverged(divergence) => {
            report.metamorphic_executed(check)?;
            let words = encode(relation.id, relation.sequence, &instance.assignment)?;
            record(
                report,
                FindingKind::MetamorphicViolation,
                SemanticFingerprint {
                    target: FuzzTarget::SpuSequence,
                    instruction_kind: None,
                    check,
                    divergence: divergence_class(divergence.first_component),
                    outcome: None,
                    effect: None,
                },
                words,
                case_index,
            )?;
            report.sequence_relation_diverged(*divergence);
            Ok(CrossReferenceAsymmetry::State)
        }
    }
}

/// The class of a divergence whose first differing component is `component`.
pub(crate) fn divergence_class(component: SpuObservationComponent) -> DivergenceClass {
    match component {
        SpuObservationComponent::ProgramCounter => DivergenceClass::ControlFlow,
        SpuObservationComponent::Effects => DivergenceClass::Effect,
        SpuObservationComponent::Outcome | SpuObservationComponent::FaultDiscard => {
            DivergenceClass::Outcome
        }
        SpuObservationComponent::Registers
        | SpuObservationComponent::LocalStore
        | SpuObservationComponent::Channels
        | SpuObservationComponent::Reservation
        | SpuObservationComponent::Fpscr
        | SpuObservationComponent::Signals
        | SpuObservationComponent::Interrupts => DivergenceClass::ArchitecturalState,
    }
}

/// The instantiated words of a row side.
pub(crate) fn encode(
    relation: SpuSequenceRelationId,
    words: &[SpuSymbolicWord],
    assignment: &[u8],
) -> Result<Vec<u32>, FuzzError> {
    words
        .iter()
        .map(|word| word.encode(assignment))
        .collect::<Option<Vec<u32>>>()
        .ok_or_else(|| InvariantError::UnencodableSequenceRelation { relation }.into())
}

/// Runs `words` and the terminator from LS address 0 of `start`.
///
/// The run then restores the program's LS words and takes PC relative to
/// the terminator, so two programs of different lengths that stop at it
/// compare equal there.
fn run_side(
    relation: SpuSequenceRelationId,
    words: &[u32],
    start: &SpuState,
) -> Result<SpuObservation, FuzzError> {
    let mut state = start.clone();
    let program: Vec<u32> = words.iter().copied().chain([TERMINATOR]).collect();
    let end = program.len() * 4;
    for (slot, word) in program.iter().enumerate() {
        state.ls[slot * 4..slot * 4 + 4].copy_from_slice(&word.to_be_bytes());
    }
    state.pc = 0;
    let mut outcome = SpuStepOutcome::Continue;
    for _ in 0..program.len() {
        let Some(raw) = state.fetch() else {
            break;
        };
        let instruction = cellgov_spu::decode::decode(raw)
            .map_err(|_| InvariantError::UnencodableSequenceRelation { relation })?;
        outcome = execute(&instruction, &mut state, UNIT);
        match &outcome {
            SpuStepOutcome::Continue => state.advance_pc(),
            SpuStepOutcome::Branch => {}
            SpuStepOutcome::Stop { kind, signal } => {
                state.record_stop(*kind, *signal);
                break;
            }
            SpuStepOutcome::Yield { .. } | SpuStepOutcome::MemoryRead { .. } => break,
            SpuStepOutcome::Fault(_) => {
                // The runtime fault-discard rule hides state from a faulting batch.
                state = start.clone();
                break;
            }
        }
    }
    state.ls[..end].copy_from_slice(&start.ls[..end]);
    state.pc = state.pc.wrapping_sub((words.len() * 4) as u32);
    Ok(SpuObservation::capture(&state, &outcome))
}

/// The registers that differ, each with the number of differing bits.
///
/// [Schkufza2013 p:308 s:4.1] The distance between two results is the
/// population count of their exclusive or, register by register.
fn bit_distance(original: &SpuObservation, partner: &SpuObservation) -> Vec<(u8, u32)> {
    original
        .state
        .regs
        .iter()
        .zip(&partner.state.regs)
        .enumerate()
        .filter_map(|(register, (left, right))| {
            let bits: u32 = left
                .iter()
                .zip(right)
                .map(|(l, r)| (l ^ r).count_ones())
                .sum();
            (bits != 0).then_some((register as u8, bits))
        })
        .collect()
}

#[cfg(test)]
#[path = "tests/sequence_relations_tests.rs"]
mod tests;
