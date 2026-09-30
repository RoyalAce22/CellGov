//! Compare rows: the negated compare, and a compare that feeds `selb` or
//! `fsm`.

use crate::instruction::SpuInstructionKind as K;
use crate::state::SpuState;

use super::lanes::{compare_mask, from_words, reg, set, words, Compare, Width};
use super::types::{
    apart, ri, rr, rrr, SpuFloatClass, SpuFusedFlow, SpuFusedReference, SpuSequencePartner,
    SpuSequenceRelation, SpuSequenceRelationId as Id, SpuSymbolicWord,
};

// Symbolic registers of the negated-compare rows, in order of appearance.
const C: u8 = 0;
const A: u8 = 1;
const B: u8 = 2;
const RT: u8 = 3;

/// [SPU-ISA p:160 s:7 Ceq] each word of RT is all ones when RA's equals RB's.
/// [SPU-ISA p:161 s:7 Ceqi] against 0, all ones exactly where `c` is zero.
pub(super) const CEQ_NOT_EQUAL: &[SpuSymbolicWord] = &[rr(K::Ceq, C, A, B), ri(K::Ceqi, RT, C, 0)];

/// [SPU-ISA p:113 s:5 Nor] `nor rt,c,c` complements `c`, which a word
/// compare leaves all ones or all zeros.
const CEQ_NOR: &[SpuSymbolicWord] = &[rr(K::Ceq, C, A, B), rr(K::Nor, RT, C, C)];

/// [SPU-ISA p:158 s:7 Ceqh] and [SPU-ISA p:159 s:7 Ceqhi]: the halfword forms.
const CEQH_NOT_EQUAL: &[SpuSymbolicWord] = &[rr(K::Ceqh, C, A, B), ri(K::Ceqhi, RT, C, 0)];

/// The fused `sext(a != b)` at `width`: `c` takes the compare, `rt` its
/// complement.
fn not_equal(state: &mut SpuState, assignment: &[u8], width: Width) {
    let equal = compare_mask(
        Compare::Equal,
        width,
        reg(state, assignment, A),
        reg(state, assignment, B),
    );
    set(state, assignment, C, equal);
    set(state, assignment, RT, equal.map(|byte| !byte));
}

pub(super) fn ceq_not_equal_fused(state: &mut SpuState, assignment: &[u8]) -> SpuFusedFlow {
    not_equal(state, assignment, Width::Word);
    SpuFusedFlow::FallThrough
}

fn ceqh_not_equal_fused(state: &mut SpuState, assignment: &[u8]) -> SpuFusedFlow {
    not_equal(state, assignment, Width::Half);
    SpuFusedFlow::FallThrough
}

/// The fused `sext(a != b)` that writes only `rt`, leaving `c` as it was.
fn ceq_not_equal_result_only(state: &mut SpuState, assignment: &[u8]) -> SpuFusedFlow {
    let kept = reg(state, assignment, C);
    ceq_not_equal_fused(state, assignment);
    if assignment[usize::from(RT)] != assignment[usize::from(C)] {
        set(state, assignment, C, kept);
    }
    SpuFusedFlow::FallThrough
}

pub(super) const CEQ_NOT_EQUAL_FUSED: SpuSequenceRelation = SpuSequenceRelation {
    id: Id::CeqNotEqualFused,
    sequence: CEQ_NOT_EQUAL,
    partner: SpuSequencePartner::Fused(SpuFusedReference {
        writes: &[C, RT],
        apply: ceq_not_equal_fused,
    }),
    precondition: None,
    float_class: SpuFloatClass::BitExact,
    dead: &[],
    pins: &[],
    local_store: false,
};

pub(super) const CEQ_NOT_EQUAL_NOR: SpuSequenceRelation = SpuSequenceRelation {
    id: Id::CeqNotEqualNor,
    sequence: CEQ_NOT_EQUAL,
    partner: SpuSequencePartner::Guest(CEQ_NOR),
    ..CEQ_NOT_EQUAL_FUSED
};

pub(super) const CEQ_NOT_EQUAL_RESULT_ONLY: SpuSequenceRelation = SpuSequenceRelation {
    id: Id::CeqNotEqualResultOnly,
    partner: SpuSequencePartner::Fused(SpuFusedReference {
        writes: &[RT],
        apply: ceq_not_equal_result_only,
    }),
    dead: &[C],
    ..CEQ_NOT_EQUAL_FUSED
};

pub(super) const CEQH_NOT_EQUAL_FUSED: SpuSequenceRelation = SpuSequenceRelation {
    id: Id::CeqhNotEqualFused,
    sequence: CEQH_NOT_EQUAL,
    partner: SpuSequencePartner::Fused(SpuFusedReference {
        writes: &[C, RT],
        apply: ceqh_not_equal_fused,
    }),
    ..CEQ_NOT_EQUAL_FUSED
};

// Symbolic registers of the select rows: `cmp c,x,y; selb rt,a,b,c`. An
// immediate compare has no `y`, so its later registers move down by one.
const SC: u8 = 0;
const SX: u8 = 1;
const SY: u8 = 2;

/// The immediate each immediate-form compare row carries: `(width, I10)`.
/// A byte compare reads I10 bits 2:9 [SPU-ISA p:157 s:7 Ceqbi].
const fn immediate(width: Width) -> i32 {
    match width {
        Width::Byte => 0x85,
        Width::Half | Width::Word => -5,
    }
}

/// The register value an immediate compare compares against.
fn immediate_value(width: Width) -> [u8; 16] {
    let imm = immediate(width);
    match width {
        Width::Word => from_words([imm as u32; 4]),
        Width::Half => {
            let half = (imm as i16 as u16).to_be_bytes();
            std::array::from_fn(|byte| half[byte % 2])
        }
        Width::Byte => [imm as u8; 16],
    }
}

/// The compare and width a select or splat row's first word applies.
const fn compare_of(kind: K) -> (Compare, Width, bool) {
    match kind {
        K::Ceq => (Compare::Equal, Width::Word, false),
        K::Ceqh => (Compare::Equal, Width::Half, false),
        K::Ceqb => (Compare::Equal, Width::Byte, false),
        K::Ceqi => (Compare::Equal, Width::Word, true),
        K::Ceqhi => (Compare::Equal, Width::Half, true),
        K::Ceqbi => (Compare::Equal, Width::Byte, true),
        K::Cgt => (Compare::Greater, Width::Word, false),
        K::Cgth => (Compare::Greater, Width::Half, false),
        K::Cgtb => (Compare::Greater, Width::Byte, false),
        K::Cgti => (Compare::Greater, Width::Word, true),
        K::Cgthi => (Compare::Greater, Width::Half, true),
        K::Cgtbi => (Compare::Greater, Width::Byte, true),
        K::Clgt => (Compare::LogicalGreater, Width::Word, false),
        K::Clgth => (Compare::LogicalGreater, Width::Half, false),
        K::Clgtb => (Compare::LogicalGreater, Width::Byte, false),
        K::Clgti => (Compare::LogicalGreater, Width::Word, true),
        K::Clgthi => (Compare::LogicalGreater, Width::Half, true),
        K::Clgtbi => (Compare::LogicalGreater, Width::Byte, true),
        K::Fceq => (Compare::FloatEqual, Width::Word, false),
        K::Fcgt => (Compare::FloatGreater, Width::Word, false),
        K::Fcmeq => (Compare::MagnitudeEqual, Width::Word, false),
        _ => (Compare::MagnitudeGreater, Width::Word, false),
    }
}

/// The select row's symbolic `(rt, a, b)`.
const fn select_registers(has_y: bool) -> (u8, u8, u8) {
    if has_y {
        (3, 4, 5)
    } else {
        (2, 3, 4)
    }
}

/// `cmp c,x,y` (or `cmp c,x,imm`) then `selb rt,a,b,c`.
///
/// [SPU-ISA p:115 s:5 Selb] each bit of RT takes RB's where RC's is one and
/// RA's where it is zero.
const fn select_sequence(kind: K) -> [SpuSymbolicWord; 2] {
    let (_, width, is_immediate) = compare_of(kind);
    let (rt, a, b) = select_registers(!is_immediate);
    let compare = if is_immediate {
        ri(kind, SC, SX, immediate(width))
    } else {
        rr(kind, SC, SX, SY)
    };
    [compare, rrr(K::Selb, rt, a, b, SC)]
}

/// The fused lane select of the row whose compare is `KIND`: `c` takes the
/// compare, and each lane of `rt` takes `b` where it holds and `a` elsewhere.
fn compare_select<const KIND: u16>(state: &mut SpuState, assignment: &[u8]) -> SpuFusedFlow {
    let kind = select_kind(KIND);
    let (compare, width, is_immediate) = compare_of(kind);
    let (rt, a, b) = select_registers(!is_immediate);
    let x = reg(state, assignment, SX);
    let y = if is_immediate {
        immediate_value(width)
    } else {
        reg(state, assignment, SY)
    };
    let mask = compare_mask(compare, width, x, y);
    let (a, b, rt) = (reg(state, assignment, a), reg(state, assignment, b), rt);
    set(state, assignment, SC, mask);
    let selected = std::array::from_fn(|byte| if mask[byte] != 0 { b[byte] } else { a[byte] });
    set(state, assignment, rt, selected);
    SpuFusedFlow::FallThrough
}

/// The compare kinds of the select rows, in row order.
const SELECT_KINDS: [K; 22] = [
    K::Ceq,
    K::Ceqh,
    K::Ceqb,
    K::Ceqi,
    K::Ceqhi,
    K::Ceqbi,
    K::Cgt,
    K::Cgth,
    K::Cgtb,
    K::Cgti,
    K::Cgthi,
    K::Cgtbi,
    K::Clgt,
    K::Clgth,
    K::Clgtb,
    K::Clgti,
    K::Clgthi,
    K::Clgtbi,
    K::Fceq,
    K::Fcgt,
    K::Fcmeq,
    K::Fcmgt,
];

const fn select_kind(index: u16) -> K {
    SELECT_KINDS[index as usize]
}

/// The select is lane-wise only while the compare's mask is not also a
/// data input: `c` may not share a register with `a` or `b`.
fn select_precondition<const KIND: u16>(_: &SpuState, assignment: &[u8]) -> bool {
    let (_, _, is_immediate) = compare_of(select_kind(KIND));
    let (_, a, b) = select_registers(!is_immediate);
    apart(assignment, &[SC], &[a, b])
}

macro_rules! select_rows {
    ($($index:literal => $id:ident, $sequence:ident;)*) => {
        $(
            const $sequence: [SpuSymbolicWord; 2] = select_sequence(select_kind($index));
        )*
        /// The compare-select rows, in [`SELECT_KINDS`] order.
        pub(super) const SELECT_ROWS: [SpuSequenceRelation; 22] = [$(
            SpuSequenceRelation {
                id: Id::$id,
                sequence: &$sequence,
                partner: SpuSequencePartner::Fused(SpuFusedReference {
                    writes: &[SC, select_registers(!compare_of(select_kind($index)).2).0],
                    apply: compare_select::<$index>,
                }),
                precondition: Some(select_precondition::<$index>),
                float_class: SpuFloatClass::BitExactUnderPrecondition,
                dead: &[],
                pins: &[],
                local_store: false,
            },
        )*];
    };
}

// [SPU-ISA p:160 s:7 Ceq] [SPU-ISA p:158 s:7 Ceqh] [SPU-ISA p:156 s:7 Ceqb]
// [SPU-ISA p:161 s:7 Ceqi] [SPU-ISA p:159 s:7 Ceqhi] [SPU-ISA p:157 s:7 Ceqbi]
// [SPU-ISA p:166 s:7 Cgt] [SPU-ISA p:164 s:7 Cgth] [SPU-ISA p:162 s:7 Cgtb]
// [SPU-ISA p:167 s:7 Cgti] [SPU-ISA p:165 s:7 Cgthi] [SPU-ISA p:163 s:7 Cgtbi]
// [SPU-ISA p:172 s:7 Clgt] [SPU-ISA p:170 s:7 Clgth] [SPU-ISA p:168 s:7 Clgtb]
// [SPU-ISA p:173 s:7 Clgti] [SPU-ISA p:171 s:7 Clgthi] [SPU-ISA p:169 s:7 Clgtbi]
// [SPU-ISA p:231 s:9 Fceq] [SPU-ISA p:233 s:9 Fcgt] [SPU-ISA p:232 s:9 Fcmeq]
// [SPU-ISA p:234 s:9 Fcmgt]
select_rows! {
    0 => SelectCeq, SELECT_CEQ;
    1 => SelectCeqh, SELECT_CEQH;
    2 => SelectCeqb, SELECT_CEQB;
    3 => SelectCeqi, SELECT_CEQI;
    4 => SelectCeqhi, SELECT_CEQHI;
    5 => SelectCeqbi, SELECT_CEQBI;
    6 => SelectCgt, SELECT_CGT;
    7 => SelectCgth, SELECT_CGTH;
    8 => SelectCgtb, SELECT_CGTB;
    9 => SelectCgti, SELECT_CGTI;
    10 => SelectCgthi, SELECT_CGTHI;
    11 => SelectCgtbi, SELECT_CGTBI;
    12 => SelectClgt, SELECT_CLGT;
    13 => SelectClgth, SELECT_CLGTH;
    14 => SelectClgtb, SELECT_CLGTB;
    15 => SelectClgti, SELECT_CLGTI;
    16 => SelectClgthi, SELECT_CLGTHI;
    17 => SelectClgtbi, SELECT_CLGTBI;
    18 => SelectFceq, SELECT_FCEQ;
    19 => SelectFcgt, SELECT_FCGT;
    20 => SelectFcmeq, SELECT_FCMEQ;
    21 => SelectFcmgt, SELECT_FCMGT;
}

// Symbolic registers of the splat rows: `cmp c,x,y; fsm rt,c`.
const SPLAT_RT: u8 = 3;

/// `cmp c,x,y; fsm rt,c`.
///
/// [SPU-ISA p:87 s:5 Fsm] each word of RT is all ones where the matching bit
/// of RA bits 28:31 is one; a word compare leaves those four bits equal.
const fn splat_sequence(kind: K) -> [SpuSymbolicWord; 2] {
    [rr(kind, SC, SX, SY), rr(K::Fsm, SPLAT_RT, SC, 0)]
}

/// The fused splat of the preferred-slot compare `KIND` indexes in
/// [`SPLAT_KINDS`]: `c` takes the compare, every word of `rt` its preferred
/// word.
fn compare_splat<const KIND: u16>(state: &mut SpuState, assignment: &[u8]) -> SpuFusedFlow {
    let (compare, width, _) = compare_of(SPLAT_KINDS[KIND as usize]);
    let mask = compare_mask(
        compare,
        width,
        reg(state, assignment, SX),
        reg(state, assignment, SY),
    );
    set(state, assignment, SC, mask);
    set(state, assignment, SPLAT_RT, from_words([words(mask)[0]; 4]));
    SpuFusedFlow::FallThrough
}

const SPLAT_KINDS: [K; 3] = [K::Ceq, K::Cgt, K::Clgt];
const SPLAT_CEQ: [SpuSymbolicWord; 2] = splat_sequence(K::Ceq);
const SPLAT_CGT: [SpuSymbolicWord; 2] = splat_sequence(K::Cgt);
const SPLAT_CLGT: [SpuSymbolicWord; 2] = splat_sequence(K::Clgt);

const fn splat_row(
    id: Id,
    sequence: &'static [SpuSymbolicWord],
    apply: fn(&mut SpuState, &[u8]) -> SpuFusedFlow,
) -> SpuSequenceRelation {
    SpuSequenceRelation {
        id,
        sequence,
        partner: SpuSequencePartner::Fused(SpuFusedReference {
            writes: &[SC, SPLAT_RT],
            apply,
        }),
        precondition: None,
        float_class: SpuFloatClass::BitExact,
        dead: &[],
        pins: &[],
        local_store: false,
    }
}

/// The splat rows, in [`SPLAT_KINDS`] order.
pub(super) const SPLAT_ROWS: [SpuSequenceRelation; 3] = [
    splat_row(Id::SplatCeq, &SPLAT_CEQ, compare_splat::<0>),
    splat_row(Id::SplatCgt, &SPLAT_CGT, compare_splat::<1>),
    splat_row(Id::SplatClgt, &SPLAT_CLGT, compare_splat::<2>),
];
