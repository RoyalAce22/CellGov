//! Guest-to-guest rows: split local-store addresses, and moves that pass a
//! value through.

use crate::instruction::SpuInstructionKind as K;
use crate::state::SpuState;

use super::lanes::{reg, words};
use super::types::{
    apart, ri, rr, SpuFloatClass, SpuSequencePartner, SpuSequenceRelation,
    SpuSequenceRelationId as Id, SpuSymbolicWord, SEQUENCE_PROGRAM_BASE, SEQUENCE_TAKEN_LANDING,
};

// Symbolic registers of the split-address rows: `ai x,y,C`, then the
// quadword access through `x` (or `y`) with `r` its data register.
const X: u8 = 0;
const Y: u8 = 1;
const R: u8 = 2;

/// The constant `ai` adds: a multiple of 16.
const C: i32 = 0x30;
/// The access's own I10, in quadwords.
const I: i32 = 2;

/// [SPU-ISA p:61 s:5 Ai] `x = y + C` per word.
/// [SPU-ISA p:32 s:3 Lqd] the address is `(I10 << 4) + RA` masked by LSLR
/// and to a quadword; [SPU-ISA p:36 s:3 Stqd] the same for a store.
const LOAD: &[SpuSymbolicWord] = &[ri(K::Ai, X, Y, C), ri(K::Lqd, R, X, I)];
const LOAD_SPLIT: &[SpuSymbolicWord] = &[ri(K::Ai, X, Y, C), ri(K::Lqd, R, Y, I + C / 16)];
const STORE: &[SpuSymbolicWord] = &[ri(K::Ai, X, Y, C), ri(K::Stqd, R, X, I)];
const STORE_SPLIT: &[SpuSymbolicWord] = &[ri(K::Ai, X, Y, C), ri(K::Stqd, R, Y, I + C / 16)];

/// The quadword the split rows access, masked as the ISA masks it.
fn access_address(state: &SpuState, assignment: &[u8]) -> u32 {
    let y = words(reg(state, assignment, Y))[0];
    y.wrapping_add(C as u32).wrapping_add((I * 16) as u32) & state.lslr() & 0xFFFF_FFF0
}

/// The split form reads `y` after `ai` writes `x`, so the two may not share
/// a register; and the access may not touch the program or the taken
/// landing, which differ between the two sides.
fn split_precondition(state: &SpuState, assignment: &[u8]) -> bool {
    let address = access_address(state, assignment);
    apart(assignment, &[X], &[Y])
        && !(SEQUENCE_PROGRAM_BASE..=SEQUENCE_TAKEN_LANDING).contains(&address)
}

pub(super) const SPLIT_LOAD_ROW: SpuSequenceRelation = SpuSequenceRelation {
    id: Id::SplitAddressLoad,
    sequence: LOAD,
    partner: SpuSequencePartner::Guest(LOAD_SPLIT),
    precondition: Some(split_precondition),
    float_class: SpuFloatClass::BitExactUnderPrecondition,
    dead: &[],
    pins: &[],
    local_store: true,
};

pub(super) const SPLIT_STORE_ROW: SpuSequenceRelation = SpuSequenceRelation {
    id: Id::SplitAddressStore,
    sequence: STORE,
    partner: SpuSequencePartner::Guest(STORE_SPLIT),
    ..SPLIT_LOAD_ROW
};

// Symbolic registers of the move rows: `move m,x`, then `a rt,m,y`.
const M: u8 = 0;
const MX: u8 = 1;
const RT: u8 = 2;
const MY: u8 = 3;

/// [SPU-ISA p:106 s:5 Ori] `ori m,x,0` passes `x` through; the add then
/// reads it [SPU-ISA p:60 s:5 A].
const ORI: &[SpuSymbolicWord] = &[ri(K::Ori, M, MX, 0), rr(K::A, RT, M, MY)];
/// [SPU-ISA p:61 s:5 Ai] adding 0.
const AI: &[SpuSymbolicWord] = &[ri(K::Ai, M, MX, 0), rr(K::A, RT, M, MY)];
/// [SPU-ISA p:101 s:5 Andi] AND with the sign-extended -1.
const ANDI: &[SpuSymbolicWord] = &[ri(K::Andi, M, MX, -1), rr(K::A, RT, M, MY)];
/// [SPU-ISA p:125 s:6 Shlqbyi] a quadword shift by 0 bytes.
const SHLQBYI: &[SpuSymbolicWord] = &[ri(K::Shlqbyi, M, MX, 0), rr(K::A, RT, M, MY)];

pub(super) const MOVE_ORI_AI_ROW: SpuSequenceRelation = SpuSequenceRelation {
    id: Id::MoveOriAi,
    sequence: ORI,
    partner: SpuSequencePartner::Guest(AI),
    precondition: None,
    float_class: SpuFloatClass::BitExact,
    dead: &[],
    pins: &[],
    local_store: false,
};

pub(super) const MOVE_ORI_ANDI_ROW: SpuSequenceRelation = SpuSequenceRelation {
    id: Id::MoveOriAndi,
    partner: SpuSequencePartner::Guest(ANDI),
    ..MOVE_ORI_AI_ROW
};

pub(super) const MOVE_ORI_SHLQBYI_ROW: SpuSequenceRelation = SpuSequenceRelation {
    id: Id::MoveOriShlqbyi,
    partner: SpuSequencePartner::Guest(SHLQBYI),
    ..MOVE_ORI_AI_ROW
};
