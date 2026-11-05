//! The builders for fixed-point, compare, branch and CR-logical
//! instructions.

use super::model::{crf, gpr, unit, Kind, Op, Target, CTR, LR, XER};
use super::storage::fx;
use crate::state::{cmp_cr_field, PpuState};
use cellgov_ps3_abi::hw::ppu_issue as k;

/// The cycles a divide of `ra` by `rb` takes, by the handbook's formula.
///
/// [CBE-Handbook p:718 s:A.1.2] PPU cycles equal
/// `((1 setup) + (ceil((rb leading digits - ra leading digits) / 2) + 1
/// iterations) + (1 fixup)) x 2`, word minimum 10 and maximum 38,
/// doubleword minimum 10 and maximum 70; overflow cases complete in 10.
fn divide_cycles(ra: u64, rb: u64, word: bool, signed: bool) -> u64 {
    let (a, b, max) = if word {
        let (a, b) = (ra as u32, rb as u32);
        let (a, b) = if signed {
            (
                (a as i32).unsigned_abs() as u64,
                (b as i32).unsigned_abs() as u64,
            )
        } else {
            (a as u64, b as u64)
        };
        (a << 32, b << 32, k::PPU_DIVIDE_WORD_MAX_CYCLES)
    } else if signed {
        (
            (ra as i64).unsigned_abs(),
            (rb as i64).unsigned_abs(),
            k::PPU_DIVIDE_DOUBLEWORD_MAX_CYCLES,
        )
    } else {
        (ra, rb, k::PPU_DIVIDE_DOUBLEWORD_MAX_CYCLES)
    };
    if b == 0 || a < b {
        return k::PPU_DIVIDE_MIN_CYCLES;
    }
    let span = u64::from(b.leading_zeros() - a.leading_zeros());
    let iterations = span.div_ceil(2) + 1;
    ((1 + iterations + 1) * 2).clamp(k::PPU_DIVIDE_MIN_CYCLES, max)
}

/// A divide; `(word, signed)` names the form.
pub(super) fn divide(
    state: &PpuState,
    rt: u8,
    ra: u8,
    rb: u8,
    oe: bool,
    rc: bool,
    form: (bool, bool),
) -> Op {
    let (word, signed) = form;
    let lat = divide_cycles(state.gpr[ra as usize], state.gpr[rb as usize], word, signed)
        + k::PPU_DIVIDE_REFETCH_CYCLES;
    let op = Op::on(unit::FXU).src(gpr(ra)).src(gpr(rb));
    let op = if oe { op.dst(XER, lat) } else { op };
    if rc {
        op.cr_recording(gpr(rt), lat)
    } else {
        op.dst(gpr(rt), lat).kind(Kind::Unpipelined(lat as u16))
    }
}

pub(super) fn multiply(ra: u8, rb: Option<u8>, rt: u8, lat: u64, rc: bool) -> Op {
    let op = Op::on(unit::FXU).src(gpr(ra));
    let op = match rb {
        Some(rb) => op.src(gpr(rb)),
        None => op,
    };
    if rc {
        op.cr_recording(gpr(rt), lat)
    } else {
        op.dst(gpr(rt), lat).kind(Kind::Unpipelined(lat as u16))
    }
}

/// A fixed-point operation of latency 2 that writes `ra`, microcoded
/// when it records.
///
/// [CBE-Handbook p:727 s:A.1.3.1] logical, count-leading-zeros,
/// extend-sign and rotate-immediate recording forms are microcoded.
pub(super) fn logical(op: Op, ra: u8, rc: bool) -> Op {
    if rc {
        op.cr_recording(gpr(ra), k::PPU_FX_LATENCY_CYCLES)
    } else {
        op.dst(gpr(ra), k::PPU_FX_LATENCY_CYCLES)
    }
}

pub(super) fn cmp(bf: u8, ra: u8, rb: Option<u8>) -> Op {
    let op = Op::on(unit::FXU).src(gpr(ra));
    let op = match rb {
        Some(rb) => op.src(gpr(rb)),
        None => op,
    };
    op.dst(crf(bf), k::PPU_COMPARE_LATENCY_CYCLES)
}

/// A branch: it reads the CR field `bi` names unless `BO` ignores the
/// condition, and reads and writes CTR when `BO` decrements it.
///
/// [PPC-Book1 p:20 s:2.4.1] `BO` bit 0 (value 16) ignores the condition
/// and bit 2 (value 4) leaves CTR alone.
pub(super) fn branch(bo: u8, bi: u8, link: bool, target: Target, taken: bool) -> Op {
    let mut op = Op::on(unit::BRU).kind(Kind::Branch {
        bo,
        link,
        target,
        taken,
    });
    if bo & 0x10 == 0 {
        op = op.src(crf(bi / 4));
    }
    if bo & 0x04 == 0 {
        op = op.src(CTR).dst(CTR, k::PPU_BRANCH_UNIT_LATENCY_CYCLES);
    }
    match target {
        Target::Link => op = op.src(LR),
        Target::Count => op = op.src(CTR),
        Target::Direct => {}
    }
    if link {
        op = op.dst(LR, k::PPU_BRANCH_UNIT_LATENCY_CYCLES);
    }
    op
}

/// Whether a conditional branch with `bo` and `bi` branches from
/// `state`.
///
/// [PPC-Book1 p:20 s:2.4.1] the branch is taken when the CTR clause
/// (decrement, then test against zero) and the condition clause (CR bit
/// `bi` against `BO` bit 1) both hold; `BO` bits 0 and 2 drop them.
pub(super) fn branches(state: &PpuState, bo: u8, bi: u8) -> bool {
    branches_on(state.cr(), state, bo, bi)
}

/// Whether the branch of a fused compare-and-`bc` branches.
///
/// The condition reads CR with field `bf` set from the compare of `a`
/// with `b`. `T` sets the width and signedness of the compare.
///
/// [PPC-Book1 p:60 s:3.3.9] the compare sets field BF to c || `XER[SO]`.
pub(super) fn fused_branches<T: Ord>(
    state: &PpuState,
    bf: u8,
    (a, b): (T, T),
    bo: u8,
    bi: u8,
) -> bool {
    let shift = u32::from(7 - bf) * 4;
    let field = u32::from(cmp_cr_field(a < b, a > b, state.xer_so()));
    let cr = (state.cr() & !(0xF << shift)) | (field << shift);
    branches_on(cr, state, bo, bi)
}

/// Whether a conditional branch with `bo` and `bi` branches when CR
/// holds `cr` and CTR is the one in `state`. [`branches`] cites the
/// two clauses.
fn branches_on(cr: u32, state: &PpuState, bo: u8, bi: u8) -> bool {
    let ctr_ok = bo & 0x04 != 0 || ((state.ctr().wrapping_sub(1) != 0) != (bo & 0x02 != 0));
    let bit = (cr >> (31 - u32::from(bi))) & 1 != 0;
    let cond_ok = bo & 0x10 != 0 || bit == (bo & 0x08 != 0);
    ctr_ok && cond_ok
}

pub(super) fn cr_logical(bt: u8, ba: u8, bb: u8) -> Op {
    Op::on(unit::BRU)
        .src(crf(ba / 4))
        .src(crf(bb / 4))
        .src(crf(bt / 4))
        .dst(crf(bt / 4), k::PPU_BRANCH_UNIT_LATENCY_CYCLES)
}

/// An add-family operation: GPR at 2, CR0 at 1 when recording, XER at 2
/// when it writes CA or OV. The decoder issues these whole.
///
/// [CBE-Handbook p:717 s:A.1.2] `add. subf. subfic neg.`: 2 (gpr),
/// 1 (cr); the carrying and extended forms 2 (xer).
pub(super) fn add_form(rt: u8, srcs: &[u8], xer: bool, rc: bool) -> Op {
    let mut op = fx(gpr(rt));
    for &s in srcs {
        op = op.src(gpr(s));
    }
    let op = if xer {
        op.dst(XER, k::PPU_FX_LATENCY_CYCLES)
    } else {
        op
    };
    op.record_cr0(rc)
}
