//! `vaddfp` and `vsubfp` through `execute_vx`, and `vmaddfp` and
//! `vnmsubfp` through `execute_va`: four independent slots, the mode from
//! `VSCR[NJ]`, and no status written.

use super::lane_support::pack_u32x4;
use crate::exec::test_support::exec_no_mem;
use crate::exec::ExecuteVerdict;
use crate::instruction::ops::{VaOp, VxOp};
use crate::instruction::PpuInstruction;
use crate::state::PpuState;
use cellgov_ps3_abi::hw::ppu::{VSCR_NJ, VSCR_SAT};

/// vD after `op` on vA, vB = `a`, `b` with `VSCR[NJ]` = `nj`; checks that
/// SAT and CR do not change.
fn run(op: VxOp, nj: bool, a: u128, b: u128) -> u128 {
    let mut s = PpuState::new();
    s.set_vscr(if nj { VSCR_NJ } else { 0 });
    s.set_cr(0xABCD_EF51);
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
    assert_eq!((s.vscr() & VSCR_SAT, s.cr()), (0, 0xABCD_EF51), "{op:?}");
    s.vr[3]
}

#[test]
fn each_slot_adds_on_its_own() {
    // 1 + 2, 1.5 + -1.5, inf + -inf, NaN + 1.
    let a = pack_u32x4([0x3F80_0000, 0x3FC0_0000, 0x7F80_0000, 0x7F81_2345]);
    let b = pack_u32x4([0x4000_0000, 0xBFC0_0000, 0xFF80_0000, 0x3F80_0000]);
    assert_eq!(
        run(VxOp::Vaddfp, false, a, b),
        pack_u32x4([0x4040_0000, 0, 0x7FC0_0000, 0x7FC1_2345])
    );
}

#[test]
fn vsubfp_takes_vb_from_va() {
    // 1 - 2, -0 - +0, inf - inf, 1 - -NaN.
    let a = pack_u32x4([0x3F80_0000, 0x8000_0000, 0x7F80_0000, 0x3F80_0000]);
    let b = pack_u32x4([0x4000_0000, 0, 0x7F80_0000, 0xFFC0_0007]);
    assert_eq!(
        run(VxOp::Vsubfp, false, a, b),
        pack_u32x4([0xBF80_0000, 0x8000_0000, 0x7FC0_0000, 0xFFC0_0007])
    );
}

/// vD after VA `op` on vA, vB, vC = `a`, `b`, `c` in registers 1, 2, 4
/// with `VSCR[NJ]` = `nj`; checks that SAT does not change.
fn run_va(op: VaOp, nj: bool, a: u128, b: u128, c: u128) -> u128 {
    let mut s = PpuState::new();
    s.set_vscr(if nj { VSCR_NJ } else { 0 });
    s.set_vr(1, a);
    s.set_vr(2, b);
    s.set_vr(4, c);
    s.set_vr(3, u128::MAX);
    let insn = PpuInstruction::Va {
        op,
        vt: 3,
        va: 1,
        vb: 2,
        vc: 4,
    };
    assert_eq!(
        exec_no_mem(&insn, &mut s),
        ExecuteVerdict::Continue,
        "{op:?}"
    );
    assert_eq!(s.vscr() & VSCR_SAT, 0, "{op:?}");
    s.vr[3]
}

/// [AltiVec-PEM p:6-71 s:6.2] vmaddfp vD, vA, vC, vB: vC multiplies and
/// vB adds.
#[test]
fn vmaddfp_multiplies_by_vc_and_adds_vb() {
    // 2 x 3 + 1, -1 x 0 + -0, inf x 0 + 1, 1 x QNaN(vC) + SNaN(vB).
    let a = pack_u32x4([0x4000_0000, 0xBF80_0000, 0x7F80_0000, 0x3F80_0000]);
    let b = pack_u32x4([0x3F80_0000, 0x8000_0000, 0x3F80_0000, 0x7F80_0005]);
    let c = pack_u32x4([0x4040_0000, 0, 0, 0x7FC0_0007]);
    assert_eq!(
        run_va(VaOp::Vmaddfp, false, a, b, c),
        pack_u32x4([0x40E0_0000, 0x8000_0000, 0x7FC0_0000, 0x7FC0_0005])
    );
    // -(2 x 3 - 1); -(-0 - -0) = -(+0); the default NaN; vB's NaN unnegated.
    assert_eq!(
        run_va(VaOp::Vnmsubfp, false, a, b, c),
        pack_u32x4([0xC0A0_0000, 0x8000_0000, 0x7FC0_0000, 0x7FC0_0005])
    );
}

#[test]
fn the_multiply_add_takes_nj_from_vscr() {
    // 1 x denormal + 0: the denormal under NJ = 0, +0 under NJ = 1, and
    // -0 from vnmsubfp under NJ = 1.
    let (one, d) = (pack_u32x4([0x3F80_0000; 4]), pack_u32x4([0x0000_0003; 4]));
    let zero = pack_u32x4([0; 4]);
    assert_eq!(run_va(VaOp::Vmaddfp, false, one, zero, d), d);
    assert_eq!(run_va(VaOp::Vmaddfp, true, one, zero, d), zero);
    assert_eq!(
        run_va(VaOp::Vnmsubfp, true, one, zero, d),
        pack_u32x4([0x8000_0000; 4])
    );
}

/// vD after a round-to-integer `op` on vB = `b`; vA holds ones, which the
/// op must ignore.
fn run_vrfi(op: VxOp, nj: bool, b: u128) -> u128 {
    let mut s = PpuState::new();
    s.set_vscr(if nj { VSCR_NJ } else { 0 });
    s.set_vr(1, u128::MAX);
    s.set_vr(2, b);
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
    s.vr[3]
}

/// [AltiVec-PEM p:6-126 s:6.2], [AltiVec-PEM p:6-128 s:6.2],
/// [AltiVec-PEM p:6-127 s:6.2], [AltiVec-PEM p:6-125 s:6.2]: the four
/// rounding modes, each on -1.5, 2.5, -0.5 and the smallest denormal.
#[test]
fn each_round_to_integer_op_takes_its_own_mode() {
    let b = pack_u32x4([0xBFC0_0000, 0x4020_0000, 0xBF00_0000, 0x0000_0001]);
    let cases = [
        (VxOp::Vrfin, [0xC000_0000, 0x4000_0000, 0x8000_0000, 0]),
        (VxOp::Vrfiz, [0xBF80_0000, 0x4000_0000, 0x8000_0000, 0]),
        (
            VxOp::Vrfip,
            [0xBF80_0000, 0x4040_0000, 0x8000_0000, 0x3F80_0000],
        ),
        (VxOp::Vrfim, [0xC000_0000, 0x4000_0000, 0xBF80_0000, 0]),
    ];
    for (op, want) in cases {
        assert_eq!(run_vrfi(op, false, b), pack_u32x4(want), "{op:?}");
    }
    // NJ = 1 reads the denormal as +0, so vrfip leaves it at +0.
    assert_eq!(
        run_vrfi(VxOp::Vrfip, true, b),
        pack_u32x4([0xBF80_0000, 0x4040_0000, 0x8000_0000, 0])
    );
}

/// [AltiVec-PEM p:6-31 s:6.2] with `VSCR[NJ]` = 1 a denormal operand or
/// result becomes a zero of its sign.
#[test]
fn vscr_nj_selects_the_denormal_rule() {
    let a = pack_u32x4([0x0000_0001, 0x00C0_0000, 0x8000_0003, 0x3F80_0000]);
    let b = pack_u32x4([0x0000_0001, 0x0080_0000, 0x8000_0001, 0x0000_0001]);
    assert_eq!(
        run(VxOp::Vaddfp, false, a, b),
        pack_u32x4([0x0000_0002, 0x0120_0000, 0x8000_0004, 0x3F80_0000])
    );
    assert_eq!(
        run(VxOp::Vsubfp, false, a, b),
        pack_u32x4([0, 0x0040_0000, 0x8000_0002, 0x3F80_0000])
    );
    assert_eq!(
        run(VxOp::Vaddfp, true, a, b),
        pack_u32x4([0, 0x0120_0000, 0x8000_0000, 0x3F80_0000])
    );
    assert_eq!(
        run(VxOp::Vsubfp, true, a, b),
        pack_u32x4([0, 0, 0, 0x3F80_0000])
    );
}
