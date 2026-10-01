//! The sequence-relation runner: one catalog row, instantiated on real
//! registers and a start state, runs as sequence A and as its partner, and
//! the runner compares the two final observations.
//!
//! The runner executes the row words directly and not through the seeded
//! hooks the other tiers own, so a seeded replay, footprint or common-mode
//! defect stays a finding of its own check. The one seeded hook it calls
//! corrupts the partner's observation, and only the sequence-partner
//! defects drive it.

use cellgov_event::UnitId;
use cellgov_spu::exec::{execute, SpuStepOutcome};
use cellgov_spu::fuzz::{
    sequence_relations, ulp_distance, SpuFloatClass, SpuFusedFlow, SpuSequencePartner,
    SpuSequencePin, SpuSequenceRelation, SpuSequenceRelationId, SpuSymbolicWord,
    SEQUENCE_PROGRAM_BASE, SEQUENCE_TAKEN_LANDING,
};
use cellgov_spu::instruction::SpuInstructionKind;
use cellgov_spu::observation::{SpuObservation, SpuObservationComponent};
use cellgov_spu::state::{SpuState, SPU_REG_COUNT};

use super::counterexample::{reduce_instance, stored_counterexamples, RelationCounterexample};
use super::record::record;
use crate::error::{FuzzError, InvariantError};
use crate::report::{
    CheckIdentity, ComponentIdentity, DivergenceClass, FindingKind, FuzzReport, FuzzTarget,
    SemanticFingerprint, SequenceRelationDivergence, StoredReplay,
};
use crate::retention::CrossReferenceAsymmetry;
use crate::rng::Rng;
use crate::seeded;
use crate::CAMPAIGN_VERSION;

const UNIT: UnitId = UnitId::new(0);

/// `stop 0x3FFE`: the terminator both sides of a relation stop at when the
/// sequence takes no branch.
/// [SPU-ISA p:238 s:10 Stop] opcode 0x000 with the signal type in bits 18:31.
pub(crate) const TERMINATOR: u32 = 0x0000_3FFE;

/// `stop 0x3FFD`: the terminator at the taken landing.
pub(crate) const TAKEN_TERMINATOR: u32 = 0x0000_3FFD;

/// Draws one case makes of a row before it leaves the row unexercised.
const PRECONDITION_DRAWS: u32 = 16;

/// Lane values the instantiation draws beside random words: zero, one, the
/// signed extremes, all ones and a halfword boundary, which are also the
/// single-precision classes +0, a denormal, the largest exponent-255 value,
/// -0 and its negative; then the smallest normal, the largest IEEE normal,
/// exponent 255 with a zero fraction, and +1.0 and -1.0.
///
/// [Aharoni2003 p:17 s:1] A float test draws its operands from the boundary
/// classes of the format.
const BOUNDARY_WORDS: [u32; 11] = [
    0,
    1,
    0x7FFF_FFFF,
    0x8000_0000,
    0xFFFF_FFFF,
    0x0000_FFFF,
    0x0080_0000,
    0x7F7F_FFFF,
    0x7F80_0000,
    0x3F80_0000,
    0xBF80_0000,
];

/// The exponents a float row's normal operands draw from.
const FLOAT_EXPONENTS: std::ops::RangeInclusive<u32> = 60..=190;

/// True for a single-precision float instruction.
fn is_float(kind: SpuInstructionKind) -> bool {
    use SpuInstructionKind as K;
    matches!(
        kind,
        K::Fa
            | K::Fs
            | K::Fm
            | K::Fma
            | K::Fms
            | K::Fnms
            | K::Frest
            | K::Frsqest
            | K::Fi
            | K::Fceq
            | K::Fcmeq
            | K::Fcgt
            | K::Fcmgt
    )
}

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
    let float_row = relation.sequence.iter().any(|word| is_float(word.kind));
    for &register in &assignment {
        let mut value = [0u8; 16];
        for lane in value.chunks_exact_mut(4) {
            let word = if float_row {
                // Mostly normal operands, so a float row's precondition
                // holds; the boundary classes still come often enough to
                // fall outside it.
                if rng.chance(1, 8)? {
                    BOUNDARY_WORDS[rng.below(BOUNDARY_WORDS.len() as u64)? as usize]
                } else {
                    let exponent = FLOAT_EXPONENTS.start()
                        + rng.below(u64::from(
                            FLOAT_EXPONENTS.end() - FLOAT_EXPONENTS.start() + 1,
                        ))? as u32;
                    // Three in four positive, so a row that needs a
                    // positive operand in every lane still compares.
                    let sign = u32::from(rng.chance(1, 4)?) << 31;
                    rng.next_u32() & 0x007F_FFFF | exponent << 23 | sign
                }
            } else if rng.chance(1, 2)? {
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
    // An all-zero register, so a test of a whole register can hold.
    if rng.chance(1, 8)? {
        let register = assignment[rng.below(count as u64)? as usize];
        start.set_reg(usize::from(register), [0; 16]);
    }
    for &(symbolic, pin) in relation.pins {
        let register = usize::from(assignment[usize::from(symbolic)]);
        let mut value = start.regs[register];
        match pin {
            SpuSequencePin::TakenLanding => {
                value[0..4].copy_from_slice(&SEQUENCE_TAKEN_LANDING.to_be_bytes());
            }
            SpuSequencePin::Word(word) => {
                for lane in value.chunks_exact_mut(4) {
                    lane.copy_from_slice(&word.to_be_bytes());
                }
            }
        }
        start.set_reg(register, value);
    }
    if relation.local_store {
        rng.fill(&mut start.ls);
    }
    Ok(RelationInstance { assignment, start })
}

/// Runs both sides of `relation` from `instance` and compares them under
/// the row's dead set.
pub(crate) fn compare_relation(
    relation: &SpuSequenceRelation,
    instance: &RelationInstance,
    case_index: u64,
) -> Result<RelationVerdict, FuzzError> {
    compare_under(
        relation,
        instance,
        case_index,
        relation.dead,
        &[],
        &|_, _| {},
    )
}

/// A change a test makes to the partner's observation before the
/// comparison: a seeded partner defect.
pub(crate) type PartnerHook<'a> = &'a dyn Fn(&mut SpuObservation, &RelationInstance);

/// Runs both sides with `tail` after each and compares them, leaving out
/// the registers the dead set `dead` excludes.
///
/// [Le2014 p:219 s:3.1.1] The partner is equivalent only over the inputs on
/// which both have a defined result. The precondition names that domain, and
/// the runner does not compare a start state outside it.
fn compare_under(
    relation: &SpuSequenceRelation,
    instance: &RelationInstance,
    case_index: u64,
    dead: &[u8],
    tail: &[u32],
    hook: PartnerHook<'_>,
) -> Result<RelationVerdict, FuzzError> {
    if let Some(precondition) = relation.precondition {
        if !precondition(&instance.start, &instance.assignment) {
            return Ok(RelationVerdict::Inapplicable);
        }
    }
    compare_sides(relation, instance, case_index, dead, tail, hook)
}

/// Runs both sides of `relation` from `instance`, with `tail` after each.
fn run_sides(
    relation: &SpuSequenceRelation,
    instance: &RelationInstance,
    tail: &[u32],
) -> Result<(SpuObservation, SpuObservation), FuzzError> {
    let mut words = encode(relation.id, relation.sequence, &instance.assignment)?;
    words.extend_from_slice(tail);
    let original = run_side(relation.id, &words, &instance.start, false)?;
    let partner = match relation.partner {
        SpuSequencePartner::Guest(partner) => {
            let mut partner = encode(relation.id, partner, &instance.assignment)?;
            partner.extend_from_slice(tail);
            run_side(relation.id, &partner, &instance.start, false)?
        }
        SpuSequencePartner::Fused(fused) => {
            let mut state = instance.start.clone();
            let flow = (fused.apply)(&mut state, &instance.assignment);
            run_side(relation.id, tail, &state, flow == SpuFusedFlow::Taken)?
        }
    };
    Ok((original, partner))
}

/// The real registers `relation` compares within its ULP bound.
fn approximate_registers(
    relation: &SpuSequenceRelation,
    instance: &RelationInstance,
) -> Vec<usize> {
    relation
        .approximate
        .iter()
        .map(|&symbolic| usize::from(instance.assignment[usize::from(symbolic)]))
        .collect()
}

/// The words of `value`.
fn lanes(value: &[u8; 16]) -> [u32; 4] {
    std::array::from_fn(|i| {
        u32::from_be_bytes([
            value[4 * i],
            value[4 * i + 1],
            value[4 * i + 2],
            value[4 * i + 3],
        ])
    })
}

/// Compares both sides without asking the precondition.
fn compare_sides(
    relation: &SpuSequenceRelation,
    instance: &RelationInstance,
    case_index: u64,
    dead: &[u8],
    tail: &[u32],
    hook: PartnerHook<'_>,
) -> Result<RelationVerdict, FuzzError> {
    let (original, mut partner) = run_sides(relation, instance, tail)?;
    hook(&mut partner, instance);
    seeded::spu_sequence_partner(
        &mut partner,
        relation,
        &instance.assignment,
        &instance.start,
    );
    Ok(judge(
        relation, instance, case_index, dead, &original, partner,
    ))
}

/// Runs sequence A of `relation` from `instance` and compares it against
/// `result`, the state a fused form leaves, under the row's precondition,
/// dead set and ULP bound. `taken` says that the fused form leaves control
/// at the taken landing.
///
/// [Martignoni2012 p:338 s:2] Two implementations differ when they start in
/// one test state and end in different final states.
pub(crate) fn compare_result_state(
    relation: &SpuSequenceRelation,
    instance: &RelationInstance,
    result: &SpuState,
    taken: bool,
) -> Result<RelationVerdict, FuzzError> {
    if let Some(precondition) = relation.precondition {
        if !precondition(&instance.start, &instance.assignment) {
            return Ok(RelationVerdict::Inapplicable);
        }
    }
    let words = encode(relation.id, relation.sequence, &instance.assignment)?;
    let original = run_side(relation.id, &words, &instance.start, false)?;
    let partner = run_side(relation.id, &[], result, taken)?;
    Ok(judge(
        relation,
        instance,
        0,
        relation.dead,
        &original,
        partner,
    ))
}

/// Compares the observation sequence A left with the partner's, leaving
/// out the registers the dead set `dead` excludes and the lanes within the
/// row's ULP bound.
fn judge(
    relation: &SpuSequenceRelation,
    instance: &RelationInstance,
    case_index: u64,
    dead: &[u8],
    original: &SpuObservation,
    mut partner: SpuObservation,
) -> RelationVerdict {
    // [Mullen2016 p:449 s:1] A dead register may hold another value; the
    // Registers comparison leaves it out, and LS, channels, PC and effects
    // stay compared.
    for register in relation.excluded_registers(&instance.assignment, dead) {
        let register = usize::from(register);
        partner.state.regs[register] = original.state.regs[register];
    }
    // [Schkufza2014 p:56 s:4] An inexact row matches when no lane is more
    // ULPs from the target than its bound; a row with no bound only
    // measures the distance.
    let bound = match relation.float_class {
        SpuFloatClass::Inexact { ulp } => ulp,
        _ => Some(0),
    };
    for register in approximate_registers(relation, instance) {
        let (left, right) = (
            lanes(&original.state.regs[register]),
            lanes(&partner.state.regs[register]),
        );
        for lane in 0..4 {
            if bound.is_none_or(|bound| ulp_distance(left[lane], right[lane]) <= bound) {
                let at = 4 * lane;
                let value = original.state.regs[register][at..at + 4].to_vec();
                partner.state.regs[register][at..at + 4].copy_from_slice(&value);
            }
        }
    }
    // [Martignoni2009 p:127 s:2.2] The comparison covers the complete state
    // after execution.
    let differences = original.compare(&partner).differences;
    let Some(&first_component) = differences.iter().next() else {
        return RelationVerdict::Match;
    };
    RelationVerdict::Diverged(Box::new(SequenceRelationDivergence {
        relation: relation.id,
        case_index,
        first_component,
        start_registers: Box::new(*instance.start.regs.as_array()),
        assignment: instance.assignment.clone(),
        bit_distance: bit_distance(original, &partner),
        differing_words: differing_words(original, &partner),
    }))
}

/// The largest ULP distance in any lane of the registers `relation`
/// compares within its bound, over the draws of `draws` the precondition
/// admits; `None` when it admits none.
///
/// [Schkufza2014 p:58 s:5.3] The largest observed sample bounds the ULP
/// error between the target and the rewrite.
///
/// # Errors
///
/// [`FuzzError`] when a draw fails or the row does not encode.
pub fn measured_ulp(
    relation: &SpuSequenceRelation,
    seed: u64,
    draws: u64,
) -> Result<Option<u32>, FuzzError> {
    let mut largest = None;
    for index in 0..draws {
        let mut rng = Rng::for_case(CAMPAIGN_VERSION, seed, index);
        let instance = instantiate(relation, &mut rng)?;
        if relation
            .precondition
            .is_some_and(|precondition| !precondition(&instance.start, &instance.assignment))
        {
            continue;
        }
        let (original, partner) = run_sides(relation, &instance, &[])?;
        for register in approximate_registers(relation, &instance) {
            let (left, right) = (
                lanes(&original.state.regs[register]),
                lanes(&partner.state.regs[register]),
            );
            for lane in 0..4 {
                let distance = ulp_distance(left[lane], right[lane]);
                largest = Some(largest.map_or(distance, |so_far: u32| so_far.max(distance)));
            }
        }
    }
    Ok(largest)
}

/// What the precondition check found wrong with one catalog row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreconditionFinding {
    /// No draw satisfied the precondition, so the check compared nothing.
    NeverInside {
        /// The row.
        relation: SpuSequenceRelationId,
    },
    /// No draw fell outside the precondition, so its need is untested.
    NeverOutside {
        /// The row.
        relation: SpuSequenceRelationId,
    },
    /// No draw outside the precondition diverges: the row holds there too,
    /// so the precondition is stronger than the rewrite needs.
    Unneeded {
        /// The row.
        relation: SpuSequenceRelationId,
    },
}

/// Checks every catalog row with a precondition over `draws` draws from
/// `seed`: the draws fall on both sides of it, and some draw outside it
/// diverges, which shows the row needs it.
///
/// [Mukherjee2024 p:120:10 s:4.3] A valid precondition is false on the
/// negative examples and true on the positive one.
/// [Mukherjee2024 p:120:14 s:5.4] It is as weak as it can be while it still
/// justifies the rewrite.
///
/// # Errors
///
/// [`FuzzError`] when a draw fails or a row does not encode.
pub fn check_preconditions(seed: u64, draws: u64) -> Result<Vec<PreconditionFinding>, FuzzError> {
    let mut findings = Vec::new();
    for relation in sequence_relations() {
        let Some(precondition) = relation.precondition else {
            continue;
        };
        let (mut inside, mut outside, mut diverged) = (0u64, 0u64, 0u64);
        for index in 0..draws {
            let mut rng = Rng::for_case(CAMPAIGN_VERSION, seed, index);
            let instance = instantiate(relation, &mut rng)?;
            if precondition(&instance.start, &instance.assignment) {
                inside += 1;
                continue;
            }
            outside += 1;
            let verdict =
                compare_sides(relation, &instance, index, relation.dead, &[], &|_, _| {})?;
            diverged += u64::from(matches!(verdict, RelationVerdict::Diverged(_)));
        }
        let relation = relation.id;
        if inside == 0 {
            findings.push(PreconditionFinding::NeverInside { relation });
        }
        if outside == 0 {
            findings.push(PreconditionFinding::NeverOutside { relation });
        } else if diverged == 0 {
            findings.push(PreconditionFinding::Unneeded { relation });
        }
    }
    Ok(findings)
}

/// The dead registers of `relation` no draw needs: with one removed from
/// the dead set, none of `draws` instantiations diverges. A dead set larger
/// than its fusion needs overstates what a recompiler may leave stale.
pub(crate) fn unneeded_dead_registers(
    relation: &SpuSequenceRelation,
    seed: u64,
    draws: u64,
) -> Result<Vec<u8>, FuzzError> {
    let mut unneeded = Vec::new();
    for &register in relation.dead {
        let reduced: Vec<u8> = relation
            .dead
            .iter()
            .copied()
            .filter(|dead| *dead != register)
            .collect();
        let mut needed = false;
        for index in 0..draws {
            let mut rng = Rng::for_case(CAMPAIGN_VERSION, seed, index);
            let instance = instantiate(relation, &mut rng)?;
            let verdict = compare_under(relation, &instance, index, &reduced, &[], &|_, _| {})?;
            if matches!(verdict, RelationVerdict::Diverged(_)) {
                needed = true;
                break;
            }
        }
        if !needed {
            unneeded.push(register);
        }
    }
    Ok(unneeded)
}

/// The excluded dead registers whose value a read after sequence A carries
/// into a live register and so into the comparison.
///
/// The tail copies each excluded register into a fresh scratch register
/// (`ori scratch, dead, 0`); a partner that left the dead register stale
/// then differs in the scratch register, which names the register read.
pub(crate) fn reader_tail_reads(
    relation: &SpuSequenceRelation,
    instance: &RelationInstance,
) -> Result<Vec<u8>, FuzzError> {
    let excluded = relation.excluded_registers(&instance.assignment, relation.dead);
    let mut assignment = instance.assignment.clone();
    let mut reads = Vec::new();
    let mut tail = Vec::new();
    for dead in excluded {
        let Some(scratch) =
            (0..SPU_REG_COUNT as u8).find(|register| !assignment.contains(register))
        else {
            break;
        };
        let (source, target) = (assignment.len() as u8, assignment.len() as u8 + 1);
        assignment.extend([dead, scratch]);
        // [SPU-ISA p:106 s:5 Ori] `ori rt,ra,0` copies RA into RT.
        let read = SpuSymbolicWord {
            kind: SpuInstructionKind::Ori,
            rt: target,
            ra: source,
            rb: 0,
            rc: 0,
            imm: 0,
        };
        tail.extend(encode(relation.id, &[read], &assignment)?);
        reads.push((dead, scratch));
    }
    let verdict = compare_under(relation, instance, 0, relation.dead, &tail, &|_, _| {})?;
    let RelationVerdict::Diverged(divergence) = verdict else {
        return Ok(Vec::new());
    };
    Ok(reads
        .into_iter()
        .filter(|(_, scratch)| {
            divergence
                .bit_distance
                .iter()
                .any(|(register, _)| register == scratch)
        })
        .map(|(dead, _)| dead)
        .collect())
}

/// What the dead-set checks found wrong with one catalog row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeadSetFinding {
    /// No draw needs this symbolic register dead: with it removed from the
    /// dead set, the partner still matches.
    Unneeded {
        /// The row.
        relation: SpuSequenceRelationId,
        /// The symbolic register.
        register: u8,
    },
    /// No draw's reader tail diverges on this symbolic register, so a
    /// later read of it cannot tell the partner from sequence A.
    NeverRead {
        /// The row.
        relation: SpuSequenceRelationId,
        /// The symbolic register.
        register: u8,
    },
}

/// Runs the minimality and reader-tail checks on every catalog row with a
/// dead set, over `draws` instantiations drawn from `seed`.
///
/// # Errors
///
/// [`FuzzError`] when a draw fails or a row does not encode.
pub fn check_dead_sets(seed: u64, draws: u64) -> Result<Vec<DeadSetFinding>, FuzzError> {
    let mut findings = Vec::new();
    for relation in sequence_relations() {
        if relation.dead.is_empty() {
            continue;
        }
        for register in unneeded_dead_registers(relation, seed, draws)? {
            findings.push(DeadSetFinding::Unneeded {
                relation: relation.id,
                register,
            });
        }
        let mut read = Vec::new();
        for index in 0..draws {
            let mut rng = Rng::for_case(CAMPAIGN_VERSION, seed, index);
            let instance = instantiate(relation, &mut rng)?;
            for real in reader_tail_reads(relation, &instance)? {
                read.extend(
                    relation
                        .dead
                        .iter()
                        .copied()
                        .filter(|&symbolic| instance.assignment[usize::from(symbolic)] == real),
                );
            }
        }
        for &register in relation.dead {
            if !read.contains(&register) {
                findings.push(DeadSetFinding::NeverRead {
                    relation: relation.id,
                    register,
                });
            }
        }
    }
    Ok(findings)
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
    // Every row takes its turn, so a bounded campaign reaches each of them.
    let relation = &relations[(case_index % relations.len() as u64) as usize];
    let check = CheckIdentity::SpuSequenceRelation(relation.id);
    // A draw outside the precondition counts as inapplicable and draws
    // again, a bounded number of times.
    let mut attempt = 0;
    let (instance, verdict) = loop {
        let instance = instantiate(relation, rng)?;
        let verdict = compare_relation(relation, &instance, case_index)?;
        attempt += 1;
        if verdict != RelationVerdict::Inapplicable || attempt == PRECONDITION_DRAWS {
            break (instance, verdict);
        }
        report.metamorphic_skipped(check)?;
    };
    match verdict {
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
                    component: Some(ComponentIdentity::Spu(divergence.first_component)),
                },
                words,
                case_index,
            )?;
            // Only a retained finding keeps a fixture, so only it pays for
            // the reduction.
            if report.retains_relation_counterexample() {
                let reduced = reduce_instance(relation, &instance, divergence.first_component)?;
                report.relation_counterexample(RelationCounterexample::from_instance(
                    format!(
                        "{:?}-{:?}-{}-{case_index}",
                        relation.id, report.strategy, report.seed
                    ),
                    relation,
                    &reduced,
                    divergence.first_component,
                ));
            }
            report.sequence_relation_diverged(*divergence);
            Ok(CrossReferenceAsymmetry::State)
        }
    }
}

/// Replays every stored counterexample and records what each shows.
///
/// [Schkufza2013 p:308 s:4.1] A counterexample from a failed check joins
/// the testcases a later check runs, and runs before any new sample.
pub(super) fn replay_stored(report: &mut FuzzReport) -> Result<(), FuzzError> {
    let stored = stored_counterexamples().map_err(|_| InvariantError::StoredCounterexamples)?;
    replay_counterexamples(report, &stored)
}

/// Replays `counterexamples` in order and records what each shows.
pub(crate) fn replay_counterexamples(
    report: &mut FuzzReport,
    counterexamples: &[RelationCounterexample],
) -> Result<(), FuzzError> {
    for counterexample in counterexamples {
        report.stored_replayed(StoredReplay {
            name: counterexample.name.clone(),
            relation: counterexample.relation,
            divergence: counterexample.replay()?,
        })?;
    }
    Ok(())
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

/// Runs `words` and the terminator from [`SEQUENCE_PROGRAM_BASE`] of
/// `start`, or from the taken landing when `taken`.
///
/// The run then restores the program's LS words and the landing's, and
/// takes PC relative to the fall-through terminator, so two programs of
/// different lengths that stop at it compare equal there; a stop at the
/// taken landing keeps its absolute PC.
fn run_side(
    relation: SpuSequenceRelationId,
    words: &[u32],
    start: &SpuState,
    taken: bool,
) -> Result<SpuObservation, FuzzError> {
    let mut state = start.clone();
    let program: Vec<u32> = words.iter().copied().chain([TERMINATOR]).collect();
    let base = SEQUENCE_PROGRAM_BASE as usize;
    let landing = SEQUENCE_TAKEN_LANDING as usize;
    let end = base + program.len() * 4;
    for (slot, word) in program.iter().enumerate() {
        let at = base + slot * 4;
        state.ls[at..at + 4].copy_from_slice(&word.to_be_bytes());
    }
    state.ls[landing..landing + 4].copy_from_slice(&TAKEN_TERMINATOR.to_be_bytes());
    state.pc = if taken {
        SEQUENCE_TAKEN_LANDING
    } else {
        SEQUENCE_PROGRAM_BASE
    };
    let mut outcome = SpuStepOutcome::Continue;
    for _ in 0..=program.len() {
        let Some(raw) = state.fetch() else {
            break;
        };
        let Ok(instruction) = cellgov_spu::decode::decode(raw) else {
            // A row word that does not decode is a catalog defect; a word
            // fetched elsewhere, after a draw outside the precondition sent
            // control there, ends the run.
            if (SEQUENCE_PROGRAM_BASE..end as u32).contains(&state.pc) {
                return Err(InvariantError::UnencodableSequenceRelation { relation }.into());
            }
            break;
        };
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
    state.ls[base..end].copy_from_slice(&start.ls[base..end]);
    state.ls[landing..landing + 4].copy_from_slice(&start.ls[landing..landing + 4]);
    if state.pc != SEQUENCE_TAKEN_LANDING + 4 {
        state.pc = state
            .pc
            .wrapping_sub(SEQUENCE_PROGRAM_BASE + (words.len() * 4) as u32);
    }
    Ok(SpuObservation::capture(&state, &outcome))
}

/// Each word that differs, as `(register, word)`: the lanes a finding
/// names.
fn differing_words(original: &SpuObservation, partner: &SpuObservation) -> Vec<(u8, u8)> {
    original
        .state
        .regs
        .iter()
        .zip(&partner.state.regs)
        .enumerate()
        .flat_map(|(register, (left, right))| {
            (0..4u8)
                .filter(move |&word| {
                    let at = usize::from(word) * 4;
                    left[at..at + 4] != right[at..at + 4]
                })
                .map(move |word| (register as u8, word))
        })
        .collect()
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
