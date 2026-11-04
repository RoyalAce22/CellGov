//! The nine integer vector compares, with and without the CR6 record,
//! against a wide-integer reference written from the PEM pseudocode.

use super::lane_support::{class_pair_vectors, lane};
use crate::exec::test_support::exec_no_mem;
use crate::exec::ExecuteVerdict;
use crate::instruction::ops::VxOp;
use crate::instruction::PpuInstruction;
use crate::state::PpuState;
use cellgov_ps3_abi::hw::ppu::VSCR_SAT;

/// Every CR field holds a value no recording compare produces in CR6.
const CR_SEED: u32 = 0xABCD_EF51;

#[derive(Clone, Copy, Debug)]
enum Rel {
    Eq,
    GtU,
    GtS,
}

const OPS: [(VxOp, u32, Rel); 9] = [
    (VxOp::Vcmpequb, 8, Rel::Eq),
    (VxOp::Vcmpequh, 16, Rel::Eq),
    (VxOp::Vcmpequw, 32, Rel::Eq),
    (VxOp::Vcmpgtub, 8, Rel::GtU),
    (VxOp::Vcmpgtuh, 16, Rel::GtU),
    (VxOp::Vcmpgtuw, 32, Rel::GtU),
    (VxOp::Vcmpgtsb, 8, Rel::GtS),
    (VxOp::Vcmpgtsh, 16, Rel::GtS),
    (VxOp::Vcmpgtsw, 32, Rel::GtS),
];

/// Runs `op` with the given registers over a CR seeded with `CR_SEED`.
fn compare(op: VxOp, rc: bool, regs: (u8, u8, u8), a: u128, b: u128) -> PpuState {
    let (vt, va, vb) = regs;
    let mut s = PpuState::new();
    s.set_cr(CR_SEED);
    s.set_vr(usize::from(va), a);
    s.set_vr(usize::from(vb), b);
    let insn = PpuInstruction::Vx { op, rc, vt, va, vb };
    assert_eq!(
        exec_no_mem(&insn, &mut s),
        ExecuteVerdict::Continue,
        "{op:?}"
    );
    s
}

/// Whether the relation holds for two `w`-bit elements.
///
/// [AltiVec-PEM p:6-54 s:6.2] vcmpequb: a vD element is all ones when the elements are equal, else all zeros.
/// [AltiVec-PEM p:6-59 s:6.2] vcmpgtsb: the same for vA greater than vB, compared signed.
/// [AltiVec-PEM p:6-62 s:6.2] vcmpgtub: the same for vA greater than vB, compared unsigned.
fn holds(w: u32, rel: Rel, a: u64, b: u64) -> bool {
    let signed = |x: u64| ((x << (64 - w)) as i64) >> (64 - w);
    match rel {
        Rel::Eq => a == b,
        Rel::GtU => a > b,
        Rel::GtS => signed(a) > signed(b),
    }
}

/// CR after a recording compare whose elements were `truths`.
///
/// [AltiVec-PEM p:6-54 s:6.2] With Rc=1, CR[24:27] = t || 0b0 || f || 0b0, t when every element is true, f when none is.
fn cr_after(truths: &[bool]) -> u32 {
    let t = truths.iter().all(|&x| x);
    let f = truths.iter().all(|&x| !x);
    let cr6 = (u32::from(t) << 3) | (u32::from(f) << 1);
    (CR_SEED & !0xF0) | (cr6 << 4)
}

#[test]
fn every_class_pair_matches_the_reference_in_both_forms() {
    for (op, w, rel) in OPS {
        let mask = (1u64 << w) - 1;
        let mut checked = 0;
        for (a, b, pairs) in class_pair_vectors(w) {
            // A short last vector holds (0, 0) in its unfilled lanes; CR6
            // reads every lane, so the truths come from all of them.
            let truths: Vec<bool> = (0..128 / w)
                .map(|i| holds(w, rel, lane(a, w, i), lane(b, w, i)))
                .collect();
            for rc in [false, true] {
                let s = compare(op, rc, (3, 1, 2), a, b);
                for (i, &t) in (0..).zip(&truths) {
                    let want = if t { mask } else { 0 };
                    assert_eq!(lane(s.vr[3], w, i), want, "{op:?} rc={rc} lane {i}");
                }
                let cr = if rc { cr_after(&truths) } else { CR_SEED };
                assert_eq!(s.cr(), cr, "{op:?} rc={rc}");
            }
            checked += pairs.len();
        }
        assert_eq!(checked, 36, "{op:?}");
    }
}

#[test]
fn cr6_reports_all_true_all_false_and_mixed() {
    for (op, w, rel) in OPS {
        let mask = (1u64 << w) - 1;
        let fill = |x: u64| (0..128 / w).fold(0u128, |v, _| (v << w) | u128::from(x));
        // Equality and greater-than each take their own all-true pair.
        let (ta, tb) = match rel {
            Rel::Eq => (5, 5),
            Rel::GtU | Rel::GtS => (6, 5),
        };
        let all_true = compare(op, true, (3, 1, 2), fill(ta), fill(tb));
        assert_eq!(all_true.vr[3], u128::MAX, "{op:?}");
        assert_eq!(all_true.cr(), (CR_SEED & !0xF0) | (0b1000 << 4), "{op:?}");

        let all_false = compare(op, true, (3, 1, 2), fill(4), fill(5));
        assert_eq!(all_false.vr[3], 0, "{op:?}");
        assert_eq!(all_false.cr(), (CR_SEED & !0xF0) | (0b0010 << 4), "{op:?}");

        // Element 0 holds, the rest do not.
        let top = u128::from(mask) << (128 - w);
        let mixed_a = (fill(4) & !top) | (u128::from(ta) << (128 - w));
        let mixed_b = (fill(5) & !top) | (u128::from(tb) << (128 - w));
        let mixed = compare(op, true, (3, 1, 2), mixed_a, mixed_b);
        assert_eq!(mixed.vr[3], top, "{op:?}");
        assert_eq!(mixed.cr(), CR_SEED & !0xF0, "{op:?}");
    }
}

#[test]
fn signedness_decides_0x80_against_0x7f() {
    let a = u128::from_be_bytes([0x80; 16]);
    let b = u128::from_be_bytes([0x7F; 16]);
    assert_eq!(
        compare(VxOp::Vcmpgtub, false, (3, 1, 2), a, b).vr[3],
        u128::MAX
    );
    assert_eq!(compare(VxOp::Vcmpgtsb, false, (3, 1, 2), a, b).vr[3], 0);
}

#[test]
fn a_register_compared_with_itself_is_equal_and_not_greater() {
    let v = 0x8000_7FFF_0123_4567_89AB_CDEF_FFFF_0000;
    for (op, _, rel) in OPS {
        let s = compare(op, true, (1, 1, 1), v, v);
        let (want, cr6) = match rel {
            Rel::Eq => (u128::MAX, 0b1000),
            Rel::GtU | Rel::GtS => (0, 0b0010),
        };
        assert_eq!(s.vr[1], want, "{op:?}");
        assert_eq!(s.cr(), (CR_SEED & !0xF0) | (cr6 << 4), "{op:?}");
    }
}

#[test]
fn a_target_that_is_also_a_source_reads_the_source_first() {
    let a = u128::from_be_bytes([6; 16]);
    let b = u128::from_be_bytes([5; 16]);
    let s = compare(VxOp::Vcmpgtub, false, (1, 1, 2), a, b);
    assert_eq!(s.vr[1], u128::MAX);
    let s = compare(VxOp::Vcmpgtub, false, (2, 1, 2), a, b);
    assert_eq!(s.vr[2], u128::MAX);
}

/// [AltiVec-PEM p:6-62 s:6.2] vcmpgtub alters CR6 when Rc=1 and no other register beside vD; the other pages say the same.
#[test]
fn no_compare_changes_sat() {
    for start_sat in [false, true] {
        for (op, ..) in OPS {
            let mut s = PpuState::new();
            if start_sat {
                s.set_sat();
            }
            s.set_vr(1, u128::MAX);
            let insn = PpuInstruction::Vx {
                op,
                rc: true,
                vt: 3,
                va: 1,
                vb: 2,
            };
            assert_eq!(exec_no_mem(&insn, &mut s), ExecuteVerdict::Continue);
            assert_eq!(s.vscr() & VSCR_SAT != 0, start_sat, "{op:?}");
        }
    }
}
