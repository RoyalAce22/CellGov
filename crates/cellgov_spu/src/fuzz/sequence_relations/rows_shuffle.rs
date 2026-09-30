//! Insertion rows: a generate-controls word feeding `shufb`.

use crate::instruction::SpuInstructionKind as K;
use crate::state::SpuState;

use super::lanes::{reg, set, words};
use super::types::{
    ri, rr, rrr, SpuFloatClass, SpuFusedFlow, SpuFusedReference, SpuSequencePartner,
    SpuSequenceRelation, SpuSequenceRelationId as Id, SpuSymbolicWord,
};

// Symbolic registers of the d-form rows: `cwd m,imm(p); shufb rt,a,b,m`.
const M: u8 = 0;
const P: u8 = 1;
// The x form reads a second address register `q` before `rt`.
const Q: u8 = 2;

/// The immediate every d-form row carries.
const IMM: i32 = 5;

/// `(rt, a, b)` of a d-form or x-form row.
const fn shuffle_registers(x_form: bool) -> (u8, u8, u8) {
    if x_form {
        (3, 4, 5)
    } else {
        (2, 3, 4)
    }
}

/// The element size a generate-controls kind inserts, and its x form flag.
///
/// [SPU-ISA p:40 s:3 Cbd] a byte, at `(RA + I7) & 0xF`; [SPU-ISA p:41 s:3 Cbx]
/// at `(RA + RB) & 0xF`. [SPU-ISA p:42 s:3 Chd] [SPU-ISA p:43 s:3 Chx] a
/// halfword, `& 0xE`. [SPU-ISA p:44 s:3 Cwd] [SPU-ISA p:45 s:3 Cwx] a word,
/// `& 0xC`. [SPU-ISA p:46 s:3 Cdd] [SPU-ISA p:47 s:3 Cdx] a doubleword, `& 0x8`.
const fn element(kind: K) -> (usize, bool) {
    match kind {
        K::Cbd => (1, false),
        K::Chd => (2, false),
        K::Cwd => (4, false),
        K::Cdd => (8, false),
        K::Cbx => (1, true),
        K::Chx => (2, true),
        K::Cwx => (4, true),
        _ => (8, true),
    }
}

/// `cXd m,imm(p)` or `cXx m,p,q`, then `shufb rt,a,b,m`.
///
/// [SPU-ISA p:116 s:5 Shufb] each result byte takes the byte of RA || RB
/// its control byte selects.
const fn insert_sequence(kind: K) -> [SpuSymbolicWord; 2] {
    let (_, x_form) = element(kind);
    let (rt, a, b) = shuffle_registers(x_form);
    let control = if x_form {
        rr(kind, M, P, Q)
    } else {
        ri(kind, M, P, IMM)
    };
    [control, rrr(K::Shufb, rt, a, b, M)]
}

const KINDS: [K; 8] = [
    K::Cbd,
    K::Chd,
    K::Cwd,
    K::Cdd,
    K::Cbx,
    K::Chx,
    K::Cwx,
    K::Cdx,
];

/// The fused insert of `a`'s preferred element into `b`. It writes the
/// control `m` first and reads `a` and `b` after it, as the sequence does,
/// so an `m` that shares a register with `a` or `b` inserts the control.
fn insert<const KIND: u8>(state: &mut SpuState, assignment: &[u8]) -> SpuFusedFlow {
    let kind = KINDS[KIND as usize];
    let (size, x_form) = element(kind);
    let (rt, a, b) = shuffle_registers(x_form);
    let base = words(reg(state, assignment, P))[0];
    let offset = if x_form {
        words(reg(state, assignment, Q))[0]
    } else {
        IMM as u32
    };
    let at = (base.wrapping_add(offset) as usize) & (0x10 - size);
    // The control selects RB's bytes (0x10..0x1F) except at the insert,
    // which takes RA's preferred element: byte 3, halfword 2:3, word 0:3,
    // doubleword 0:7.
    let first = 4usize.saturating_sub(size);
    let control: [u8; 16] = std::array::from_fn(|byte| {
        if (at..at + size).contains(&byte) {
            (first + byte - at) as u8
        } else {
            0x10 + byte as u8
        }
    });
    set(state, assignment, M, control);
    let (a, b) = (reg(state, assignment, a), reg(state, assignment, b));
    let mut result = b;
    result[at..at + size].copy_from_slice(&a[first..first + size]);
    set(state, assignment, rt, result);
    SpuFusedFlow::FallThrough
}

const CBD: [SpuSymbolicWord; 2] = insert_sequence(K::Cbd);
const CHD: [SpuSymbolicWord; 2] = insert_sequence(K::Chd);
const CWD: [SpuSymbolicWord; 2] = insert_sequence(K::Cwd);
const CDD: [SpuSymbolicWord; 2] = insert_sequence(K::Cdd);
const CBX: [SpuSymbolicWord; 2] = insert_sequence(K::Cbx);
const CHX: [SpuSymbolicWord; 2] = insert_sequence(K::Chx);
const CWX: [SpuSymbolicWord; 2] = insert_sequence(K::Cwx);
const CDX: [SpuSymbolicWord; 2] = insert_sequence(K::Cdx);

/// What a d-form and an x-form fusion write: `m`, then `rt`.
const D_WRITES: &[u8] = &[M, shuffle_registers(false).0];
const X_WRITES: &[u8] = &[M, shuffle_registers(true).0];

const fn insert_row(
    id: Id,
    sequence: &'static [SpuSymbolicWord],
    x_form: bool,
    apply: fn(&mut SpuState, &[u8]) -> SpuFusedFlow,
) -> SpuSequenceRelation {
    SpuSequenceRelation {
        id,
        sequence,
        partner: SpuSequencePartner::Fused(SpuFusedReference {
            writes: if x_form { X_WRITES } else { D_WRITES },
            apply,
        }),
        precondition: None,
        float_class: SpuFloatClass::BitExact,
        dead: &[],
        pins: &[],
        local_store: false,
        approximate: &[],
    }
}

/// The insertion rows, in [`KINDS`] order.
pub(super) const INSERT_ROWS: [SpuSequenceRelation; 8] = [
    insert_row(Id::InsertCbd, &CBD, false, insert::<0>),
    insert_row(Id::InsertChd, &CHD, false, insert::<1>),
    insert_row(Id::InsertCwd, &CWD, false, insert::<2>),
    insert_row(Id::InsertCdd, &CDD, false, insert::<3>),
    insert_row(Id::InsertCbx, &CBX, true, insert::<4>),
    insert_row(Id::InsertChx, &CHX, true, insert::<5>),
    insert_row(Id::InsertCwx, &CWX, true, insert::<6>),
    insert_row(Id::InsertCdx, &CDX, true, insert::<7>),
];
