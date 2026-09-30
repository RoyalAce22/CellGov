//! Sequence relations: an SPU instruction sequence and a partner that must
//! leave the same observed state from the same start state.
//!
//! A partner is another guest sequence, or a fused reference: a typed
//! operation on the start state that writes a named set of registers, the
//! form a recompiler emits for a fused sequence. Rows name registers
//! symbolically, numbered in order of first appearance, so one row covers
//! every register assignment.

use crate::instruction::SpuInstructionKind;
use crate::state::SpuState;

use super::classify::form_for_kind;
use super::metamorphic::opcode_word;
use super::types::SpuEncodingForm;

/// One sequence relation of the catalog.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum SpuSequenceRelationId {
    /// `ceq c,a,b; ceqi rt,c,0` against the fused `sext(a != b)`, which
    /// also writes `c`.
    CeqNotEqualFused,
    /// `ceq c,a,b; ceqi rt,c,0` against `ceq c,a,b; nor rt,c,c`.
    CeqNotEqualNor,
}

/// How exactly a relation's partner matches its sequence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpuFloatClass {
    /// Bit-exact on every start state.
    BitExact,
    /// Bit-exact on every start state the precondition admits.
    BitExactUnderPrecondition,
    /// Inexact: the partner may differ in low-order bits.
    Inexact,
}

/// One instruction of a relation row, with symbolic registers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpuSymbolicWord {
    /// The instruction kind.
    pub kind: SpuInstructionKind,
    /// Symbolic RT.
    pub rt: u8,
    /// Symbolic RA.
    pub ra: u8,
    /// Symbolic RB; unused by an immediate form.
    pub rb: u8,
    /// The immediate field value; unused by a register form.
    pub imm: u32,
}

impl SpuSymbolicWord {
    /// The word with each symbolic register replaced by `assignment[index]`;
    /// `None` for an encoding form the rows do not use, or an index past
    /// the assignment.
    pub fn encode(&self, assignment: &[u8]) -> Option<u32> {
        let register = |index: u8| assignment.get(usize::from(index)).map(|r| u32::from(*r));
        let opcode = opcode_word(self.kind)?;
        let (rt, ra) = (register(self.rt)?, register(self.ra)?);
        Some(match form_for_kind(self.kind) {
            SpuEncodingForm::Rrr => opcode | register(self.rb)? << 14 | ra << 7 | rt,
            SpuEncodingForm::Ri10 => opcode | (self.imm & 0x3FF) << 14 | ra << 7 | rt,
            SpuEncodingForm::Ri7 => opcode | (self.imm & 0x7F) << 14 | ra << 7 | rt,
            _ => return None,
        })
    }
}

/// A fused partner: an operation on the start state, and the symbolic
/// registers it writes.
#[derive(Debug, Clone, Copy)]
pub struct SpuFusedReference {
    /// The symbolic registers the operation writes.
    pub writes: &'static [u8],
    /// Applies the operation to `state` under `assignment`.
    pub apply: fn(&mut SpuState, &[u8]),
}

/// The partner a relation compares its sequence against.
#[derive(Debug, Clone, Copy)]
pub enum SpuSequencePartner {
    /// Another guest sequence.
    Guest(&'static [SpuSymbolicWord]),
    /// A fused reference.
    Fused(SpuFusedReference),
}

/// A start-state test under `assignment`: true when the relation claims
/// the partner matches.
pub type SpuSequencePrecondition = fn(&SpuState, &[u8]) -> bool;

/// One row of the sequence-relation catalog.
#[derive(Debug, Clone, Copy)]
pub struct SpuSequenceRelation {
    /// The row's identity.
    pub id: SpuSequenceRelationId,
    /// Sequence A.
    pub sequence: &'static [SpuSymbolicWord],
    /// Partner B.
    pub partner: SpuSequencePartner,
    /// The start states the claim covers; `None` covers every state.
    pub precondition: Option<SpuSequencePrecondition>,
    /// How exactly B matches A.
    pub float_class: SpuFloatClass,
}

impl SpuSequenceRelation {
    /// The number of symbolic registers the row names.
    pub fn register_count(&self) -> usize {
        let partner: &[SpuSymbolicWord] = match self.partner {
            SpuSequencePartner::Guest(words) => words,
            SpuSequencePartner::Fused(_) => &[],
        };
        let fused_writes = match self.partner {
            SpuSequencePartner::Fused(fused) => fused.writes,
            SpuSequencePartner::Guest(_) => &[],
        };
        self.sequence
            .iter()
            .chain(partner)
            .flat_map(|word| [word.rt, word.ra, word.rb])
            .chain(fused_writes.iter().copied())
            .max()
            .map_or(0, |highest| usize::from(highest) + 1)
    }
}

const fn word(kind: SpuInstructionKind, rt: u8, ra: u8, rb: u8, imm: u32) -> SpuSymbolicWord {
    SpuSymbolicWord {
        kind,
        rt,
        ra,
        rb,
        imm,
    }
}

// Symbolic registers of the compare rows, in order of appearance.
const C: u8 = 0;
const A: u8 = 1;
const B: u8 = 2;
const RT: u8 = 3;

/// [SPU-ISA p:160 s:7 Ceq] each word of RT is all ones when RA's equals RB's.
/// [SPU-ISA p:161 s:7 Ceqi] against 0, all ones exactly where `c` is zero.
const CEQ_NOT_EQUAL: &[SpuSymbolicWord] = &[
    word(SpuInstructionKind::Ceq, C, A, B, 0),
    word(SpuInstructionKind::Ceqi, RT, C, 0, 0),
];

/// [SPU-ISA p:113 s:5 Nor] `nor rt,c,c` complements `c`, which a word
/// compare leaves all ones or all zeros.
const CEQ_NOR: &[SpuSymbolicWord] = &[
    word(SpuInstructionKind::Ceq, C, A, B, 0),
    word(SpuInstructionKind::Nor, RT, C, C, 0),
];

/// The fused `sext(a != b)`: `c` takes the word compare, `rt` its complement.
fn ceq_not_equal_fused(state: &mut SpuState, assignment: &[u8]) {
    let (a, b) = (
        state.regs[usize::from(assignment[usize::from(A)])],
        state.regs[usize::from(assignment[usize::from(B)])],
    );
    let equal: [u8; 16] = std::array::from_fn(|byte| {
        let word = byte / 4 * 4;
        if a[word..word + 4] == b[word..word + 4] {
            0xFF
        } else {
            0
        }
    });
    state.set_reg(usize::from(assignment[usize::from(C)]), equal);
    state.set_reg(
        usize::from(assignment[usize::from(RT)]),
        equal.map(|byte| !byte),
    );
}

const RELATIONS: &[SpuSequenceRelation] = &[
    SpuSequenceRelation {
        id: SpuSequenceRelationId::CeqNotEqualFused,
        sequence: CEQ_NOT_EQUAL,
        partner: SpuSequencePartner::Fused(SpuFusedReference {
            writes: &[C, RT],
            apply: ceq_not_equal_fused,
        }),
        precondition: None,
        float_class: SpuFloatClass::BitExact,
    },
    SpuSequenceRelation {
        id: SpuSequenceRelationId::CeqNotEqualNor,
        sequence: CEQ_NOT_EQUAL,
        partner: SpuSequencePartner::Guest(CEQ_NOR),
        precondition: None,
        float_class: SpuFloatClass::BitExact,
    },
];

/// Every sequence relation, in identity order.
pub fn sequence_relations() -> &'static [SpuSequenceRelation] {
    RELATIONS
}

#[cfg(test)]
#[path = "tests/sequence_relations_tests.rs"]
mod tests;
