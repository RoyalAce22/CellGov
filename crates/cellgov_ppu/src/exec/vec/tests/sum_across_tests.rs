//! The five sum-across instructions and the SAT they set, against a
//! wide-integer reference written from the PEM pseudocode.

use super::lane_support::pack_u32x4;
use crate::exec::test_support::exec_no_mem;
use crate::exec::ExecuteVerdict;
use crate::instruction::ops::VxOp;
use crate::instruction::PpuInstruction;
use crate::state::PpuState;
use cellgov_ps3_abi::hw::ppu::VSCR_SAT;

const OPS: [VxOp; 5] = [
    VxOp::Vsumsws,
    VxOp::Vsum2sws,
    VxOp::Vsum4sbs,
    VxOp::Vsum4shs,
    VxOp::Vsum4ubs,
];

/// Runs `op` with vD preset to all ones from a state whose SAT is `sat`;
/// returns vD and SAT after.
fn run(op: VxOp, sat: bool, a: u128, b: u128) -> (u128, bool) {
    let mut s = PpuState::new();
    if sat {
        s.set_sat();
    }
    s.set_vr(1, a);
    s.set_vr(2, b);
    s.set_vr(3, u128::MAX);
    let insn = PpuInstruction::Vx {
        op,
        rc: false,
        vt: 3,
        va: 1,
        vb: 2,
    };
    assert_eq!(
        exec_no_mem(&insn, &mut s),
        ExecuteVerdict::Continue,
        "{op:?}"
    );
    (s.vr[3], s.vscr() & VSCR_SAT != 0)
}

fn word(v: u128, i: usize) -> u64 {
    ((v >> (96 - 32 * i)) as u64) & 0xFFFF_FFFF
}

fn sx(x: u64, w: u32) -> i64 {
    ((x << (64 - w)) as i64) >> (64 - w)
}

/// `v` saturated to a 32-bit word, and whether it saturated.
fn sat32(v: i64, signed: bool) -> (u64, bool) {
    let (min, max) = if signed {
        (-(1i64 << 31), (1i64 << 31) - 1)
    } else {
        (0, (1i64 << 32) - 1)
    };
    let r = v.clamp(min, max);
    ((r as u64) & 0xFFFF_FFFF, r != v)
}

/// The four words of vD and whether any saturated.
///
/// [AltiVec-PEM p:6-166 s:6.2] vsumsws: vD = 96 zero bits || SItoSIsat(vB[96:127] + the four vA words, 32).
/// [AltiVec-PEM p:6-167 s:6.2] vsum2sws: each doubleword of vD = 32 zero bits || SItoSIsat(its odd vB word + its two vA words, 32).
/// [AltiVec-PEM p:6-168 s:6.2] vsum4sbs: each vD word = SItoSIsat(the vB word + the four signed bytes of the vA word, 32).
/// [AltiVec-PEM p:6-169 s:6.2] vsum4shs: each vD word = SItoSIsat(the vB word + the two signed half words of the vA word, 32).
/// [AltiVec-PEM p:6-170 s:6.2] vsum4ubs: each vD word = UItoUIsat(the vB word + the four unsigned bytes of the vA word, 32).
fn reference(op: VxOp, a: u128, b: u128) -> ([u64; 4], bool) {
    let mut out = [0u64; 4];
    let mut any = false;
    let mut put = |i: usize, (v, s): (u64, bool)| {
        out[i] = v;
        any |= s;
    };
    match op {
        VxOp::Vsumsws => {
            let total: i64 = (0..4).map(|i| sx(word(a, i), 32)).sum::<i64>() + sx(word(b, 3), 32);
            put(3, sat32(total, true));
        }
        VxOp::Vsum2sws => {
            for k in [1, 3] {
                let total = sx(word(a, k - 1), 32) + sx(word(a, k), 32) + sx(word(b, k), 32);
                put(k, sat32(total, true));
            }
        }
        _ => {
            for i in 0..4 {
                let (x, y) = (word(a, i), word(b, i));
                let total = match op {
                    VxOp::Vsum4sbs => {
                        (0..4).map(|j| sx(x >> (24 - 8 * j), 8)).sum::<i64>() + sx(y, 32)
                    }
                    VxOp::Vsum4shs => sx(x >> 16, 16) + sx(x, 16) + sx(y, 32),
                    _ => {
                        (0..4)
                            .map(|j| ((x >> (24 - 8 * j)) & 0xFF) as i64)
                            .sum::<i64>()
                            + y as i64
                    }
                };
                put(i, sat32(total, op != VxOp::Vsum4ubs));
            }
        }
    }
    (out, any)
}

/// Word classes: 0, 1, all ones, the signed max and min, and a fixed
/// arbitrary value. Each fills a whole word, which the op splits.
const CLASSES: [u32; 6] = [0, 1, 0xFFFF_FFFF, 0x7FFF_FFFF, 0x8000_0000, 0x9E37_79B9];

#[test]
fn every_class_combination_matches_the_reference_and_its_sat() {
    for op in OPS {
        let mut checked = 0;
        for &x in &CLASSES {
            for &y in &CLASSES {
                for &z in &CLASSES {
                    // vA rotates three classes through its words; vB holds a
                    // fourth pattern so each word of it is distinct.
                    let a = pack_u32x4([x, y, z, x ^ y]);
                    let b = pack_u32x4([z, x, y, y ^ z]);
                    let (r, s) = run(op, false, a, b);
                    let (want, any) = reference(op, a, b);
                    let got: [u64; 4] = std::array::from_fn(|i| word(r, i));
                    assert_eq!(got, want, "{op:?} vA {a:#034x} vB {b:#034x}");
                    assert_eq!(s, any, "{op:?} SAT vA {a:#034x} vB {b:#034x}");
                    checked += 1;
                }
            }
        }
        assert_eq!(checked, 216, "{op:?}");
    }
}

#[test]
fn the_full_sums_clear_the_words_they_do_not_write_and_ignore_the_vb_words_they_do_not_read() {
    let a = pack_u32x4([1, 2, 3, 4]);
    let b = pack_u32x4([10, 20, 30, 40]);
    let noisy_b = pack_u32x4([0xFFFF_FFFF, 20, 0xFFFF_FFFF, 40]);

    let (r, _) = run(VxOp::Vsumsws, false, a, b);
    assert_eq!(r, pack_u32x4([0, 0, 0, 50]));
    assert_eq!(
        run(
            VxOp::Vsumsws,
            false,
            a,
            pack_u32x4([u32::MAX, u32::MAX, u32::MAX, 40])
        )
        .0,
        r
    );

    let (r, _) = run(VxOp::Vsum2sws, false, a, b);
    assert_eq!(r, pack_u32x4([0, 23, 0, 47]));
    assert_eq!(run(VxOp::Vsum2sws, false, a, noisy_b).0, r);
}

#[test]
fn each_sum_clamps_and_sets_sat() {
    let max = pack_u32x4([0x7FFF_FFFF; 4]);
    // Four 0x7FFF_FFFF words sum past 32 bits before the clamp.
    assert_eq!(
        run(VxOp::Vsumsws, false, max, 0),
        (pack_u32x4([0, 0, 0, 0x7FFF_FFFF]), true)
    );
    assert_eq!(
        run(VxOp::Vsumsws, false, pack_u32x4([0x8000_0000; 4]), 0),
        (pack_u32x4([0, 0, 0, 0x8000_0000]), true)
    );

    let top = pack_u32x4([0xFFFF_FFFF; 4]);
    assert_eq!(
        run(VxOp::Vsum4ubs, false, pack_u32x4([0x0000_0001; 4]), top),
        (top, true)
    );
    assert_eq!(run(VxOp::Vsum4ubs, false, 0, top), (top, false));

    assert_eq!(
        run(VxOp::Vsum4sbs, false, pack_u32x4([0x7F7F_7F7F; 4]), max),
        (max, true)
    );
    let min = pack_u32x4([0x8000_0000; 4]);
    assert_eq!(
        run(VxOp::Vsum4sbs, false, pack_u32x4([0x8080_8080; 4]), min),
        (min, true)
    );
}

#[test]
fn a_target_that_is_both_sources_reads_them_first() {
    let v = pack_u32x4([1, 2, 3, 4]);
    for op in OPS {
        let mut s = PpuState::new();
        s.set_vr(1, v);
        let insn = PpuInstruction::Vx {
            op,
            rc: false,
            vt: 1,
            va: 1,
            vb: 1,
        };
        assert_eq!(exec_no_mem(&insn, &mut s), ExecuteVerdict::Continue);
        let (want, _) = reference(op, v, v);
        let got: [u64; 4] = std::array::from_fn(|i| word(s.vr[1], i));
        assert_eq!(got, want, "{op:?}");
    }
}

/// [AltiVec-PEM p:2-5 s:2.3.2] SAT stays set until mtvscr clears it.
#[test]
fn sat_set_before_stays_set() {
    for op in OPS {
        assert!(run(op, true, 0, 0).1, "{op:?}");
    }
}
