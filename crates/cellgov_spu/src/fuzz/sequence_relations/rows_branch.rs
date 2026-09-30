//! Branch rows: `orx` feeding a conditional branch, compared on the branch
//! target and on the value `orx` leaves.

use crate::instruction::SpuInstructionKind as K;
use crate::state::SpuState;

use super::lanes::{from_words, reg, set, words};
use super::types::{
    apart, branch_to_landing, rr, SpuFloatClass, SpuFusedFlow, SpuFusedReference,
    SpuSequencePartner, SpuSequencePin, SpuSequenceRelation, SpuSequenceRelationId as Id,
    SpuSymbolicWord,
};

// Symbolic registers: `orx o,v`, then the branch on `o`; an indirect
// branch takes its target from `t`.
const O: u8 = 0;
const V: u8 = 1;
const T: u8 = 2;

/// [SPU-ISA p:107 s:5 Orx] RT's preferred word is the OR of RA's four
/// words; its other words are zero.
const fn orx() -> SpuSymbolicWord {
    rr(K::Orx, O, V, 0)
}

/// [SPU-ISA p:183 s:7 Brz] taken when RT's preferred word is zero.
const BRZ: &[SpuSymbolicWord] = &[orx(), branch_to_landing(K::Brz, O, 1)];
/// [SPU-ISA p:182 s:7 Brnz] taken when it is not zero.
const BRNZ: &[SpuSymbolicWord] = &[orx(), branch_to_landing(K::Brnz, O, 1)];
/// [SPU-ISA p:186 s:7 Biz] to RA's preferred word when RT's is zero.
const BIZ: &[SpuSymbolicWord] = &[orx(), rr(K::Biz, O, T, 0)];
/// [SPU-ISA p:187 s:7 Binz] to RA's preferred word when RT's is not zero.
const BINZ: &[SpuSymbolicWord] = &[orx(), rr(K::Binz, O, T, 0)];

/// The fused test of `v`: `o` takes the OR, and the fused form branches when
/// the OR is zero (`TAKEN_ON_ZERO`) or not zero.
fn test_or<const TAKEN_ON_ZERO: bool>(state: &mut SpuState, assignment: &[u8]) -> SpuFusedFlow {
    let or = words(reg(state, assignment, V))
        .iter()
        .fold(0, |acc, word| acc | word);
    set(state, assignment, O, from_words([or, 0, 0, 0]));
    if (or == 0) == TAKEN_ON_ZERO {
        SpuFusedFlow::Taken
    } else {
        SpuFusedFlow::FallThrough
    }
}

/// The indirect branch reads its target after the sequence writes `o`.
fn indirect_precondition(_: &SpuState, assignment: &[u8]) -> bool {
    apart(assignment, &[O], &[T])
}

const fn branch_row(
    id: Id,
    sequence: &'static [SpuSymbolicWord],
    apply: fn(&mut SpuState, &[u8]) -> SpuFusedFlow,
    indirect: bool,
) -> SpuSequenceRelation {
    SpuSequenceRelation {
        id,
        sequence,
        partner: SpuSequencePartner::Fused(SpuFusedReference {
            writes: &[O],
            apply,
        }),
        precondition: if indirect {
            Some(indirect_precondition)
        } else {
            None
        },
        float_class: SpuFloatClass::BitExact,
        dead: &[],
        pins: if indirect {
            &[(T, SpuSequencePin::TakenLanding)]
        } else {
            &[]
        },
        local_store: false,
    }
}

/// The branch rows.
pub(super) const BRANCH_ROWS: [SpuSequenceRelation; 4] = [
    branch_row(Id::BranchOrxBrz, BRZ, test_or::<true>, false),
    branch_row(Id::BranchOrxBrnz, BRNZ, test_or::<false>, false),
    branch_row(Id::BranchOrxBiz, BIZ, test_or::<true>, true),
    branch_row(Id::BranchOrxBinz, BINZ, test_or::<false>, true),
];
