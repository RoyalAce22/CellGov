//! Integer rows: the 32-bit lane multiply, popcount, negated shift counts
//! and the funnel shift.

use crate::instruction::SpuInstructionKind as K;
use crate::state::SpuState;

use super::lanes::{from_halves, from_words, halves, reg, rotate_left_128, set, words};
use super::types::{
    apart, distinct, ri, rr, SpuFloatClass, SpuFusedFlow, SpuFusedReference, SpuSequencePartner,
    SpuSequenceRelation, SpuSequenceRelationId as Id, SpuSymbolicWord,
};

// Symbolic registers of the multiply rows, in order of appearance.
const T1: u8 = 0;
const MA: u8 = 1;
const MB: u8 = 2;
const T2: u8 = 3;
const S: u8 = 4;
const U: u8 = 5;
const MRT: u8 = 6;

/// [SPU-ISA p:77 s:5 Mpyh] RA's high halfword times RB's low halfword,
/// shifted left 16. [SPU-ISA p:73 s:5 Mpyu] the unsigned product of the low
/// halfwords. [SPU-ISA p:60 s:5 A] word add.
const MPY32: &[SpuSymbolicWord] = &[
    rr(K::Mpyh, T1, MA, MB),
    rr(K::Mpyh, T2, MB, MA),
    rr(K::A, S, T1, T2),
    rr(K::Mpyu, U, MA, MB),
    rr(K::A, MRT, S, U),
];

/// [`MPY32`] with the final add's operands swapped.
const MPY32_SWAPPED: &[SpuSymbolicWord] = &[
    rr(K::Mpyh, T1, MA, MB),
    rr(K::Mpyh, T2, MB, MA),
    rr(K::A, S, T1, T2),
    rr(K::Mpyu, U, MA, MB),
    rr(K::A, MRT, U, S),
];

/// The fused 32-bit multiply: each intermediate, then `rt = a * b` per word.
fn mpy32(state: &mut SpuState, assignment: &[u8]) -> SpuFusedFlow {
    let (a, b) = (
        words(reg(state, assignment, MA)),
        words(reg(state, assignment, MB)),
    );
    let high_low = |x: u32, y: u32| ((x >> 16) * (y & 0xFFFF)) << 16;
    let t1 = std::array::from_fn(|i| high_low(a[i], b[i]));
    let t2 = std::array::from_fn(|i| high_low(b[i], a[i]));
    let s: [u32; 4] = std::array::from_fn(|i| t1[i].wrapping_add(t2[i]));
    let u = std::array::from_fn(|i| (a[i] & 0xFFFF) * (b[i] & 0xFFFF));
    set(state, assignment, T1, from_words(t1));
    set(state, assignment, T2, from_words(t2));
    set(state, assignment, S, from_words(s));
    set(state, assignment, U, from_words(u));
    set(
        state,
        assignment,
        MRT,
        from_words(std::array::from_fn(|i| a[i].wrapping_mul(b[i]))),
    );
    SpuFusedFlow::FallThrough
}

/// The fusion reads `a` and `b` throughout: no intermediate may share a
/// register with them or with another intermediate.
fn mpy32_precondition(_: &SpuState, assignment: &[u8]) -> bool {
    distinct(assignment, &[T1, T2, S, U]) && apart(assignment, &[T1, T2, S, U], &[MA, MB])
}

const MPY32_WRITES: &[u8] = &[T1, T2, S, U, MRT];

pub(super) const MPY32_ROW: SpuSequenceRelation = SpuSequenceRelation {
    id: Id::Mpy32,
    sequence: MPY32,
    partner: SpuSequencePartner::Fused(SpuFusedReference {
        writes: MPY32_WRITES,
        apply: mpy32,
    }),
    precondition: Some(mpy32_precondition),
    float_class: SpuFloatClass::BitExactUnderPrecondition,
    dead: &[],
    pins: &[],
    local_store: false,
    approximate: &[],
};

pub(super) const MPY32_SWAPPED_ROW: SpuSequenceRelation = SpuSequenceRelation {
    id: Id::Mpy32Swapped,
    sequence: MPY32_SWAPPED,
    ..MPY32_ROW
};

// Symbolic registers of the negated-count rows: `sfi n,x,0; rotm rt,a,n`.
const N: u8 = 0;
const X: u8 = 1;
const NRT: u8 = 2;
const NA: u8 = 3;

/// `neg n,x,0; shift rt,a,n`.
///
/// [SPU-ISA p:65 s:5 Sfi] `sfi n,x,0` is `0 - x` per word.
/// [SPU-ISA p:63 s:5 Sfhi] `sfhi n,x,0` is `0 - x` per halfword; the word
/// negate would carry a borrow into the high halfword.
const fn negated(negate: K, shift: K) -> [SpuSymbolicWord; 2] {
    [ri(negate, N, X, 0), rr(shift, NRT, NA, N)]
}

const ROTM: [SpuSymbolicWord; 2] = negated(K::Sfi, K::Rotm);
const ROTMA: [SpuSymbolicWord; 2] = negated(K::Sfi, K::Rotma);
const ROTHM: [SpuSymbolicWord; 2] = negated(K::Sfhi, K::Rothm);
const ROTMAH: [SpuSymbolicWord; 2] = negated(K::Sfhi, K::Rotmah);
const ROTQMBI: [SpuSymbolicWord; 2] = negated(K::Sfi, K::Rotqmbi);
const ROTQMBY: [SpuSymbolicWord; 2] = negated(K::Sfi, K::Rotqmby);

/// The right shift `SHIFT` names, by `x` directly; `n` takes the negate.
///
/// [SPU-ISA p:138 s:6 Rotm] a logical word shift by `(0 - n) & 0x3F`, zero
/// at 32 and above. [SPU-ISA p:147 s:6 Rotma] the arithmetic form fills
/// with the sign. [SPU-ISA p:136 s:6 Rothm] and [SPU-ISA p:145 s:6 Rotmah]:
/// the halfword forms, count `& 0x1F`, 16 and above. [SPU-ISA p:143 s:6
/// Rotqmbi] the quadword shifts by bits `& 7` of the preferred word;
/// [SPU-ISA p:140 s:6 Rotqmby] by bytes `& 0x1F`, zero at 16 and above.
fn shift_by_x<const SHIFT: u8>(state: &mut SpuState, assignment: &[u8]) -> SpuFusedFlow {
    let x = reg(state, assignment, X);
    let a = reg(state, assignment, NA);
    let halfword = matches!(SHIFT, 2 | 3);
    let n = if halfword {
        from_halves(halves(x).map(|h| 0u16.wrapping_sub(h)))
    } else {
        from_words(words(x).map(|w| 0u32.wrapping_sub(w)))
    };
    let rt = match SHIFT {
        0 | 1 => {
            let (a, x) = (words(a), words(x));
            from_words(std::array::from_fn(|i| {
                let count = x[i] & 0x3F;
                match (SHIFT, count) {
                    (0, 32..) => 0,
                    (0, _) => a[i] >> count,
                    (_, 32..) => ((a[i] as i32) >> 31) as u32,
                    _ => ((a[i] as i32) >> count) as u32,
                }
            }))
        }
        2 | 3 => {
            let (a, x) = (halves(a), halves(x));
            from_halves(std::array::from_fn(|i| {
                let count = x[i] & 0x1F;
                match (SHIFT, count) {
                    (2, 16..) => 0,
                    (2, _) => a[i] >> count,
                    (_, 16..) => ((a[i] as i16) >> 15) as u16,
                    _ => ((a[i] as i16) >> count) as u16,
                }
            }))
        }
        4 => (u128::from_be_bytes(a) >> (words(x)[0] & 7)).to_be_bytes(),
        _ => {
            let count = words(x)[0] & 0x1F;
            if count >= 16 {
                [0; 16]
            } else {
                (u128::from_be_bytes(a) >> (count * 8)).to_be_bytes()
            }
        }
    };
    set(state, assignment, N, n);
    set(state, assignment, NRT, rt);
    SpuFusedFlow::FallThrough
}

/// The shift reads `a` after the negate writes `n`: `n` may not share its
/// register.
fn negated_precondition(_: &SpuState, assignment: &[u8]) -> bool {
    apart(assignment, &[N], &[NA])
}

const fn negated_row(
    id: Id,
    sequence: &'static [SpuSymbolicWord],
    apply: fn(&mut SpuState, &[u8]) -> SpuFusedFlow,
) -> SpuSequenceRelation {
    SpuSequenceRelation {
        id,
        sequence,
        partner: SpuSequencePartner::Fused(SpuFusedReference {
            writes: &[N, NRT],
            apply,
        }),
        precondition: Some(negated_precondition),
        float_class: SpuFloatClass::BitExactUnderPrecondition,
        dead: &[],
        pins: &[],
        local_store: false,
        approximate: &[],
    }
}

/// The negated-count rows.
pub(super) const NEGATED_ROWS: [SpuSequenceRelation; 6] = [
    negated_row(Id::NegatedCountRotm, &ROTM, shift_by_x::<0>),
    negated_row(Id::NegatedCountRotma, &ROTMA, shift_by_x::<1>),
    negated_row(Id::NegatedCountRothm, &ROTHM, shift_by_x::<2>),
    negated_row(Id::NegatedCountRotmah, &ROTMAH, shift_by_x::<3>),
    negated_row(Id::NegatedCountRotqmbi, &ROTQMBI, shift_by_x::<4>),
    negated_row(Id::NegatedCountRotqmby, &ROTQMBY, shift_by_x::<5>),
];

// Symbolic registers of the funnel row: `rotqbybi v,x,s; rotqbi rt,v,s`.
const V: u8 = 0;
const FX: u8 = 1;
const FS: u8 = 2;
const FRT: u8 = 3;

/// [SPU-ISA p:133 s:6 Rotqbybi] a byte rotate by RB bits 25:28 of the
/// preferred word (the RTL's bits 24:28 rotate the same, mod 16 bytes).
/// [SPU-ISA p:134 s:6 Rotqbi] a bit rotate by bits 29:31.
const FUNNEL: &[SpuSymbolicWord] = &[rr(K::Rotqbybi, V, FX, FS), rr(K::Rotqbi, FRT, V, FS)];

/// The fused rotate of `x` left by `s & 0x7F` bits; `v` takes the byte step.
fn funnel(state: &mut SpuState, assignment: &[u8]) -> SpuFusedFlow {
    let x = reg(state, assignment, FX);
    let count = words(reg(state, assignment, FS))[0];
    set(
        state,
        assignment,
        V,
        rotate_left_128(x, ((count >> 3) & 0xF) * 8),
    );
    set(state, assignment, FRT, rotate_left_128(x, count & 0x7F));
    SpuFusedFlow::FallThrough
}

/// `rotqbi` reads `s` after `rotqbybi` writes `v`.
fn funnel_precondition(_: &SpuState, assignment: &[u8]) -> bool {
    apart(assignment, &[V], &[FS])
}

pub(super) const FUNNEL_ROW: SpuSequenceRelation = SpuSequenceRelation {
    id: Id::FunnelShift,
    sequence: FUNNEL,
    partner: SpuSequencePartner::Fused(SpuFusedReference {
        writes: &[V, FRT],
        apply: funnel,
    }),
    precondition: Some(funnel_precondition),
    float_class: SpuFloatClass::BitExactUnderPrecondition,
    dead: &[],
    pins: &[],
    local_store: false,
    approximate: &[],
};

// Symbolic registers of the popcount row: `cntb c,a; sumb rt,c,c`.
const PC: u8 = 0;
const PA: u8 = 1;
const PRT: u8 = 2;

/// [SPU-ISA p:84 s:5 Cntb] each byte counts its one bits.
/// [SPU-ISA p:93 s:5 Sumb] each word's halfword 0 sums RB's bytes of that
/// word, halfword 1 RA's; with both `c`, both halves are the word's count.
const POPCOUNT: &[SpuSymbolicWord] = &[rr(K::Cntb, PC, PA, 0), rr(K::Sumb, PRT, PC, PC)];

/// The fused per-word popcount, with `c` the per-byte counts.
fn popcount(state: &mut SpuState, assignment: &[u8]) -> SpuFusedFlow {
    let a = reg(state, assignment, PA);
    let counts = a.map(|byte| byte.count_ones() as u8);
    let per_word = words(a).map(|word| word.count_ones() << 16 | word.count_ones());
    set(state, assignment, PC, counts);
    set(state, assignment, PRT, from_words(per_word));
    SpuFusedFlow::FallThrough
}

pub(super) const POPCOUNT_ROW: SpuSequenceRelation = SpuSequenceRelation {
    id: Id::Popcount,
    sequence: POPCOUNT,
    partner: SpuSequencePartner::Fused(SpuFusedReference {
        writes: &[PC, PRT],
        apply: popcount,
    }),
    precondition: None,
    float_class: SpuFloatClass::BitExact,
    dead: &[],
    pins: &[],
    local_store: false,
    approximate: &[],
};
