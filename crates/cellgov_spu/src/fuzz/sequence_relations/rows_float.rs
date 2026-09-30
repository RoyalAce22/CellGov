//! Float rows: estimate refinement, Newton steps, square root and division
//! against exact or host values, and compare-and-select against a host
//! minimum or maximum.
//!
//! [Menendez2016 p:318 s:1] A float rewrite is bit-exact, bit-exact in the
//! absence of special values, or inexact; each row names its class, and the
//! special values it excludes are its precondition.
//! [Mueller2005 p:61 s:3.1] SPU single precision is not IEEE: exponent 255
//! is an ordinary number, a denormal operand reads as zero, and results
//! truncate.
//! [Monniaux2008 p:9 s:3.1.1] A host float result depends on the precision
//! and rounding the host uses, so a host stand-in is exact only on part of
//! the input space.

use crate::instruction::SpuInstructionKind as K;
use crate::state::SpuState;

use super::lanes::{
    compare_mask, exponent, from_words, ieee_divide, ieee_order, ieee_sqrt, reg, set,
    truncated_reciprocal, truncated_rsqrt, words, Compare, Width,
};
use super::types::{
    distinct, rr, rrr, SpuFloatClass, SpuFusedFlow, SpuFusedReference, SpuSequencePartner,
    SpuSequencePin, SpuSequenceRelation, SpuSequenceRelationId as Id, SpuSymbolicWord,
};

const ONE: u32 = 0x3F80_0000;
const ONE_PLUS: u32 = 0x3F80_0001;
const HALF: u32 = 0x3F00_0000;
const ABS_MASK: u32 = 0x7FFF_FFFF;

/// Applies `op` to each word of symbolic `x`, writing `rt`.
fn lanewise(state: &mut SpuState, assignment: &[u8], x: u8, rt: u8, op: impl Fn(u32) -> u32) {
    let value = words(reg(state, assignment, x)).map(op);
    set(state, assignment, rt, from_words(value));
}

/// Every symbolic register of a row names its own register.
fn all_distinct(assignment: &[u8]) -> bool {
    let symbolic: Vec<u8> = (0..assignment.len() as u8).collect();
    distinct(assignment, &symbolic)
}

/// Each word of symbolic `x` has an exponent in `range`.
fn exponents_in(
    state: &SpuState,
    assignment: &[u8],
    x: u8,
    range: std::ops::RangeInclusive<u32>,
) -> bool {
    words(reg(state, assignment, x))
        .iter()
        .all(|word| range.contains(&exponent(*word)))
}

// Estimate refinement: `frest t,x; fi rt,x,t`.
const ET: u8 = 0;
const EX: u8 = 1;
const ERT: u8 = 2;

/// [SPU-ISA p:215 s:9 Frest] a base and step for the reciprocal.
/// [SPU-ISA p:219 s:9 Fi] interpolation between them by RA's fraction bits.
const ESTIMATE_RECIPROCAL: &[SpuSymbolicWord] = &[rr(K::Frest, ET, EX, 0), rr(K::Fi, ERT, EX, ET)];
/// [SPU-ISA p:217 s:9 Frsqest] a base and step for `1/sqrt(|x|)`.
const ESTIMATE_RSQRT: &[SpuSymbolicWord] = &[rr(K::Frsqest, ET, EX, 0), rr(K::Fi, ERT, EX, ET)];

fn reciprocal_of_x(state: &mut SpuState, assignment: &[u8]) -> SpuFusedFlow {
    lanewise(state, assignment, EX, ERT, |x| {
        truncated_reciprocal(x).unwrap_or(0)
    });
    SpuFusedFlow::FallThrough
}

fn rsqrt_of_x(state: &mut SpuState, assignment: &[u8]) -> SpuFusedFlow {
    lanewise(state, assignment, EX, ERT, |x| {
        truncated_rsqrt(x).unwrap_or(0)
    });
    SpuFusedFlow::FallThrough
}

/// A nonzero exponent with `|x| < 2^126`: the Normal range of the
/// reciprocal [SPU-ISA p:216 s:9 Frest], where no divide-by-zero flag is set.
fn reciprocal_domain(state: &SpuState, assignment: &[u8]) -> bool {
    all_distinct(assignment) && exponents_in(state, assignment, EX, 1..=252)
}

/// A nonzero exponent below 255: the Normal range of the reciprocal square
/// root [SPU-ISA p:218 s:9 Frsqest], less the maximal exponent, where the
/// sequence sets the DIFF flag [SPU-ISA p:196 s:9] and a host value does not.
fn rsqrt_domain(state: &SpuState, assignment: &[u8]) -> bool {
    all_distinct(assignment) && exponents_in(state, assignment, EX, 1..=254)
}

const fn estimate_row(
    id: Id,
    sequence: &'static [SpuSymbolicWord],
    apply: fn(&mut SpuState, &[u8]) -> SpuFusedFlow,
    precondition: fn(&SpuState, &[u8]) -> bool,
) -> SpuSequenceRelation {
    SpuSequenceRelation {
        id,
        sequence,
        partner: SpuSequencePartner::Fused(SpuFusedReference {
            writes: &[ERT],
            apply,
        }),
        precondition: Some(precondition),
        // The ISA states a bound only after a Newton step, so this row
        // measures the estimate's distance.
        float_class: SpuFloatClass::Inexact { ulp: None },
        dead: &[ET],
        pins: &[],
        local_store: false,
        approximate: &[ERT],
    }
}

pub(super) const ESTIMATE_RECIPROCAL_ROW: SpuSequenceRelation = estimate_row(
    Id::EstimateReciprocal,
    ESTIMATE_RECIPROCAL,
    reciprocal_of_x,
    reciprocal_domain,
);
pub(super) const ESTIMATE_RSQRT_ROW: SpuSequenceRelation =
    estimate_row(Id::EstimateRsqrt, ESTIMATE_RSQRT, rsqrt_of_x, rsqrt_domain);

// Newton reciprocal: `frest y0,d; fi y,d,y0; fnms e,d,y,one; fma rt,e,y,y`.
const Y0: u8 = 0;
const ND: u8 = 1;
const NY: u8 = 2;
const NE: u8 = 3;
const NONE: u8 = 4;
const NRT: u8 = 5;

/// [SPU-ISA p:215 s:9 Frest] the documented sequence: estimate, interpolate,
/// `e = -(d * y - one)` [SPU-ISA p:210 s:9 Fnms], `rt = e * y + y`
/// [SPU-ISA p:208 s:9 Fma].
const NEWTON: &[SpuSymbolicWord] = &[
    rr(K::Frest, Y0, ND, 0),
    rr(K::Fi, NY, ND, Y0),
    rrr(K::Fnms, NE, ND, NY, NONE),
    rrr(K::Fma, NRT, NE, NY, NY),
];

fn reciprocal_of_d(state: &mut SpuState, assignment: &[u8]) -> SpuFusedFlow {
    lanewise(state, assignment, ND, NRT, |d| {
        truncated_reciprocal(d).unwrap_or(0)
    });
    SpuFusedFlow::FallThrough
}

fn newton_domain(state: &SpuState, assignment: &[u8]) -> bool {
    all_distinct(assignment) && exponents_in(state, assignment, ND, 1..=252)
}

/// [SPU-ISA p:216 s:9 Frest] after one Newton step `|Y - y2| <= 1 ulp` of
/// the truncated reciprocal `Y`.
pub(super) const NEWTON_RECIPROCAL_ROW: SpuSequenceRelation = SpuSequenceRelation {
    id: Id::NewtonReciprocal,
    sequence: NEWTON,
    partner: SpuSequencePartner::Fused(SpuFusedReference {
        writes: &[NRT],
        apply: reciprocal_of_d,
    }),
    precondition: Some(newton_domain),
    float_class: SpuFloatClass::Inexact { ulp: Some(1) },
    dead: &[Y0, NY, NE],
    pins: &[(NONE, SpuSequencePin::Word(ONE))],
    local_store: false,
    approximate: &[NRT],
};

/// The same step with `one` one ulp above 1.0; the ISA states its bound for
/// 1.0, so this row measures its distance.
pub(super) const NEWTON_RECIPROCAL_ONE_PLUS_ROW: SpuSequenceRelation = SpuSequenceRelation {
    id: Id::NewtonReciprocalOnePlus,
    float_class: SpuFloatClass::Inexact { ulp: None },
    pins: &[(NONE, SpuSequencePin::Word(ONE_PLUS))],
    ..NEWTON_RECIPROCAL_ROW
};

// Reciprocal square root Newton step, as the ISA writes it.
const RAX: u8 = 0;
const RX: u8 = 1;
const RMASK: u8 = 2;
const RY0: u8 = 3;
const RY1: u8 = 4;
const RT1: u8 = 5;
const RT2: u8 = 6;
const RHALF: u8 = 7;
const RT3: u8 = 8;
const RONE: u8 = 9;
const RRT: u8 = 10;

/// [SPU-ISA p:217 s:9 Frsqest] the documented sequence: `ax = |x|`
/// [SPU-ISA p:97 s:5 And], estimate, interpolate, `t1 = ax * y1`,
/// `t2 = y1 * 0.5` [SPU-ISA p:206 s:9 Fm], `t3 = -(t1 * y1 - 1)`,
/// `rt = t3 * t2 + y1`.
const RSQRT_NEWTON: &[SpuSymbolicWord] = &[
    rr(K::And, RAX, RX, RMASK),
    rr(K::Frsqest, RY0, RX, 0),
    rr(K::Fi, RY1, RAX, RY0),
    rr(K::Fm, RT1, RAX, RY1),
    rr(K::Fm, RT2, RY1, RHALF),
    rrr(K::Fnms, RT3, RT1, RY1, RONE),
    rrr(K::Fma, RRT, RT3, RT2, RY1),
];

fn rsqrt_of_rx(state: &mut SpuState, assignment: &[u8]) -> SpuFusedFlow {
    lanewise(state, assignment, RX, RRT, |x| {
        truncated_rsqrt(x).unwrap_or(0)
    });
    SpuFusedFlow::FallThrough
}

fn rsqrt_newton_domain(state: &SpuState, assignment: &[u8]) -> bool {
    all_distinct(assignment) && exponents_in(state, assignment, RX, 1..=254)
}

/// [SPU-ISA p:218 s:9 Frsqest] after one Newton step `|Y - y2| <= 1 ulp` of
/// the truncated reciprocal square root `Y`.
pub(super) const RSQRT_NEWTON_ROW: SpuSequenceRelation = SpuSequenceRelation {
    id: Id::RsqrtNewton,
    sequence: RSQRT_NEWTON,
    partner: SpuSequencePartner::Fused(SpuFusedReference {
        writes: &[RRT],
        apply: rsqrt_of_rx,
    }),
    precondition: Some(rsqrt_newton_domain),
    float_class: SpuFloatClass::Inexact { ulp: Some(1) },
    dead: &[RAX, RY0, RY1, RT1, RT2, RT3],
    pins: &[
        (RMASK, SpuSequencePin::Word(ABS_MASK)),
        (RHALF, SpuSequencePin::Word(HALF)),
        (RONE, SpuSequencePin::Word(ONE)),
    ],
    local_store: false,
    approximate: &[RRT],
};

// Square root: `frsqest y0,x; fi y,x,y0; fm g,y,x; fm h,g,half;
// fnms t,y,g,one; fma rt,t,h,g`.
const SY0: u8 = 0;
const SX: u8 = 1;
const SY: u8 = 2;
const SG: u8 = 3;
const SH: u8 = 4;
const SHALF: u8 = 5;
const ST: u8 = 6;
const SONE: u8 = 7;
const SRT: u8 = 8;

/// `g = y * x` is about `sqrt(x)`; the last two steps refine it.
const SQUARE_ROOT: &[SpuSymbolicWord] = &[
    rr(K::Frsqest, SY0, SX, 0),
    rr(K::Fi, SY, SX, SY0),
    rr(K::Fm, SG, SY, SX),
    rr(K::Fm, SH, SG, SHALF),
    rrr(K::Fnms, ST, SY, SG, SONE),
    rrr(K::Fma, SRT, ST, SH, SG),
];

/// The host's single-precision `sqrt(|x|)`.
fn host_sqrt(state: &mut SpuState, assignment: &[u8]) -> SpuFusedFlow {
    lanewise(state, assignment, SX, SRT, |x| {
        ieee_sqrt(x & ABS_MASK).unwrap_or(0)
    });
    SpuFusedFlow::FallThrough
}

/// A positive normal `x` below 2^128. For a negative `x`, `g = y * x` is
/// negative, the correction `1 - y * g` is near 2 rather than near 0, and
/// the chain is far from `sqrt(|x|)`; the host has no exponent 255.
fn square_root_domain(state: &SpuState, assignment: &[u8]) -> bool {
    all_distinct(assignment)
        && words(reg(state, assignment, SX))
            .iter()
            .all(|x| x >> 31 == 0 && (1..=254).contains(&exponent(*x)))
}

/// The ISA states no bound for this chain; the bound is the largest
/// distance a validation run over the positive domain observed.
/// [Schkufza2014 p:58 s:5.3] The largest observed sample bounds the ULP
/// error between the target and the rewrite.
pub(super) const SQUARE_ROOT_ROW: SpuSequenceRelation = SpuSequenceRelation {
    id: Id::SquareRoot,
    sequence: SQUARE_ROOT,
    partner: SpuSequencePartner::Fused(SpuFusedReference {
        writes: &[SRT],
        apply: host_sqrt,
    }),
    precondition: Some(square_root_domain),
    float_class: SpuFloatClass::Inexact { ulp: Some(2) },
    dead: &[SY0, SY, SG, SH, ST],
    pins: &[
        (SHALF, SpuSequencePin::Word(HALF)),
        (SONE, SpuSequencePin::Word(ONE)),
    ],
    local_store: false,
    approximate: &[SRT],
};

// Division: `frest y0,b; fi y,b,y0; fm q,a,y; fnms t,q,b,a; fma rt,t,y,q`.
const DY0: u8 = 0;
const DB: u8 = 1;
const DY: u8 = 2;
const DQ: u8 = 3;
const DA: u8 = 4;
const DT: u8 = 5;
const DRT: u8 = 6;

/// `q = a * y` is about `a / b`; `t = a - q * b` corrects it.
const DIVISION: &[SpuSymbolicWord] = &[
    rr(K::Frest, DY0, DB, 0),
    rr(K::Fi, DY, DB, DY0),
    rr(K::Fm, DQ, DA, DY),
    rrr(K::Fnms, DT, DQ, DB, DA),
    rrr(K::Fma, DRT, DT, DY, DQ),
];

/// The host's single-precision `a / b`.
fn host_divide(state: &mut SpuState, assignment: &[u8]) -> SpuFusedFlow {
    let (a, b) = (
        words(reg(state, assignment, DA)),
        words(reg(state, assignment, DB)),
    );
    let quotient = std::array::from_fn(|i| ieee_divide(a[i], b[i]).unwrap_or(0));
    set(state, assignment, DRT, from_words(quotient));
    SpuFusedFlow::FallThrough
}

/// Normal `a` and `b` in the host's range, `b` in the reciprocal's Normal
/// range, and a quotient that stays in range. The correction `a - q * b`
/// is about 2^-24 of `a` and can cancel further, so `a` stays above 2^-77,
/// where that term would underflow and set UNF and DIFF [SPU-ISA p:196 s:9].
fn division_domain(state: &SpuState, assignment: &[u8]) -> bool {
    let (a, b) = (
        words(reg(state, assignment, DA)),
        words(reg(state, assignment, DB)),
    );
    all_distinct(assignment)
        && (0..4).all(|i| {
            let (ea, eb) = (exponent(a[i]), exponent(b[i]));
            (50..=254).contains(&ea)
                && (1..=252).contains(&eb)
                && (2..=253).contains(&(ea + 127).saturating_sub(eb))
        })
}

pub(super) const DIVISION_ROW: SpuSequenceRelation = SpuSequenceRelation {
    id: Id::Division,
    sequence: DIVISION,
    partner: SpuSequencePartner::Fused(SpuFusedReference {
        writes: &[DRT],
        apply: host_divide,
    }),
    precondition: Some(division_domain),
    float_class: SpuFloatClass::Inexact { ulp: None },
    dead: &[DY0, DY, DQ, DT],
    pins: &[],
    local_store: false,
    approximate: &[DRT],
};

// Compare and pick: `cmp c,a,b; selb rt,b,a,c` picks `a` where the compare
// holds (the max form), `selb rt,a,b,c` picks `b` there (the min form).
const PC: u8 = 0;
const PA: u8 = 1;
const PB: u8 = 2;
const PRT: u8 = 3;

/// [SPU-ISA p:233 s:9 Fcgt] [SPU-ISA p:234 s:9 Fcmgt] [SPU-ISA p:231 s:9
/// Fceq] feeding [SPU-ISA p:115 s:5 Selb].
const fn pick(compare: K, picks_a: bool) -> [SpuSymbolicWord; 2] {
    let select = if picks_a {
        rrr(K::Selb, PRT, PB, PA, PC)
    } else {
        rrr(K::Selb, PRT, PA, PB, PC)
    };
    [rr(compare, PC, PA, PB), select]
}

const FLOAT_MAX: [SpuSymbolicWord; 2] = pick(K::Fcgt, true);
const FLOAT_MIN: [SpuSymbolicWord; 2] = pick(K::Fcgt, false);
const MAGNITUDE_MAX: [SpuSymbolicWord; 2] = pick(K::Fcmgt, true);
const MAGNITUDE_MIN: [SpuSymbolicWord; 2] = pick(K::Fcmgt, false);
const EQUAL_PICK: [SpuSymbolicWord; 2] = pick(K::Fceq, true);

/// The host's pick of `a` or `b` per word, written to `rt`. A host orders
/// words by sign and magnitude, a denormal above zero [`ieee_order`].
fn host_pick(state: &mut SpuState, assignment: &[u8], choose: impl Fn(u32, u32) -> u32) {
    let (a, b) = (
        words(reg(state, assignment, PA)),
        words(reg(state, assignment, PB)),
    );
    let picked = std::array::from_fn(|i| choose(a[i], b[i]));
    set(state, assignment, PRT, from_words(picked));
}

/// True for a word a host reads as NaN: exponent 255 with a nonzero
/// fraction. [SPU-ISA p:196 s:9] IEEE arithmetic treats a maximal exponent
/// as NaN or infinity, where the SPU reads an ordinary number.
fn host_nan(bits: u32) -> bool {
    exponent(bits) == 255 && bits & 0x7F_FFFF != 0
}

/// The host's ordered `a > b`: false when either is a NaN.
fn host_greater(a: u32, b: u32) -> bool {
    !host_nan(a) && !host_nan(b) && ieee_order(a) > ieee_order(b)
}

/// The host maximum: the other operand when one is a NaN, else `a` when it
/// orders above `b`.
fn host_max(state: &mut SpuState, assignment: &[u8]) -> SpuFusedFlow {
    host_pick(state, assignment, |a, b| match (host_nan(a), host_nan(b)) {
        (true, _) => b,
        (false, true) => a,
        _ if host_greater(a, b) => a,
        _ => b,
    });
    SpuFusedFlow::FallThrough
}

/// The host minimum: the other operand when one is a NaN, else `b` when `a`
/// orders above it.
fn host_min(state: &mut SpuState, assignment: &[u8]) -> SpuFusedFlow {
    host_pick(state, assignment, |a, b| match (host_nan(a), host_nan(b)) {
        (true, _) => b,
        (false, true) => a,
        _ if host_greater(a, b) => b,
        _ => a,
    });
    SpuFusedFlow::FallThrough
}

/// The host's `|a| > |b| ? a : b`.
fn host_magnitude_max(state: &mut SpuState, assignment: &[u8]) -> SpuFusedFlow {
    host_pick(state, assignment, |a, b| {
        if host_greater(a & ABS_MASK, b & ABS_MASK) {
            a
        } else {
            b
        }
    });
    SpuFusedFlow::FallThrough
}

/// The host's `|a| > |b| ? b : a`.
fn host_magnitude_min(state: &mut SpuState, assignment: &[u8]) -> SpuFusedFlow {
    host_pick(state, assignment, |a, b| {
        if host_greater(a & ABS_MASK, b & ABS_MASK) {
            b
        } else {
            a
        }
    });
    SpuFusedFlow::FallThrough
}

/// The host equality: the same bits other than a NaN, or two zeros of
/// either sign.
fn host_equal_pick(state: &mut SpuState, assignment: &[u8]) -> SpuFusedFlow {
    host_pick(state, assignment, |a, b| {
        if (a == b && !host_nan(a)) || (a | b) & ABS_MASK == 0 {
            a
        } else {
            b
        }
    });
    SpuFusedFlow::FallThrough
}

/// No lane of `a` or `b` has exponent 255, which the host reads as an
/// infinity or NaN, and no lane pairs two zero exponents, which the SPU
/// compares equal whatever their sign and fraction [SPU-ISA p:231 s:9 Fceq].
///
/// [Mukherjee2024 p:120:14 s:5.4] A precondition is as weak as it can be
/// while it still justifies the rewrite; each clause here excludes a lane
/// class where the host and the SPU disagree.
pub(super) fn host_pick_domain(state: &SpuState, assignment: &[u8]) -> bool {
    let (a, b) = (
        words(reg(state, assignment, PA)),
        words(reg(state, assignment, PB)),
    );
    distinct(assignment, &[PC, PA, PB])
        && (0..4).all(|i| {
            let (ea, eb) = (exponent(a[i]), exponent(b[i]));
            ea != 255 && eb != 255 && !(ea == 0 && eb == 0)
        })
}

const fn pick_row(
    id: Id,
    sequence: &'static [SpuSymbolicWord],
    apply: fn(&mut SpuState, &[u8]) -> SpuFusedFlow,
) -> SpuSequenceRelation {
    SpuSequenceRelation {
        id,
        sequence,
        partner: SpuSequencePartner::Fused(SpuFusedReference {
            writes: &[PRT],
            apply,
        }),
        precondition: Some(host_pick_domain),
        float_class: SpuFloatClass::BitExactUnderPrecondition,
        dead: &[PC],
        pins: &[],
        local_store: false,
        approximate: &[],
    }
}

/// The SPU lane select the max form computes: bit-exact on every state.
fn spu_max(state: &mut SpuState, assignment: &[u8]) -> SpuFusedFlow {
    let (a, b) = (reg(state, assignment, PA), reg(state, assignment, PB));
    let mask = compare_mask(Compare::FloatGreater, Width::Word, a, b);
    set(state, assignment, PC, mask);
    let picked = std::array::from_fn(|byte| if mask[byte] != 0 { a[byte] } else { b[byte] });
    set(state, assignment, PRT, picked);
    SpuFusedFlow::FallThrough
}

/// The select reads `a` and `b` after the compare writes `c`.
fn spu_max_domain(_: &SpuState, assignment: &[u8]) -> bool {
    distinct(assignment, &[PC, PA]) && distinct(assignment, &[PC, PB])
}

/// The float rows.
pub(super) const PICK_ROWS: [SpuSequenceRelation; 6] = [
    pick_row(Id::FloatMax, &FLOAT_MAX, host_max),
    pick_row(Id::FloatMin, &FLOAT_MIN, host_min),
    pick_row(Id::MagnitudeMax, &MAGNITUDE_MAX, host_magnitude_max),
    pick_row(Id::MagnitudeMin, &MAGNITUDE_MIN, host_magnitude_min),
    pick_row(Id::EqualPick, &EQUAL_PICK, host_equal_pick),
    SpuSequenceRelation {
        id: Id::FloatMaxSelect,
        sequence: &FLOAT_MAX,
        partner: SpuSequencePartner::Fused(SpuFusedReference {
            writes: &[PC, PRT],
            apply: spu_max,
        }),
        precondition: Some(spu_max_domain),
        float_class: SpuFloatClass::BitExactUnderPrecondition,
        dead: &[],
        pins: &[],
        local_store: false,
        approximate: &[],
    },
];
