//! `vaddfp` and `vsubfp` through `execute_vx`: four independent slots,
//! the mode from `VSCR[NJ]`, and no status written.

use super::lane_support::pack_u32x4;
use crate::exec::test_support::exec_no_mem;
use crate::exec::ExecuteVerdict;
use crate::instruction::ops::VxOp;
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
