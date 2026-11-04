//! `vctsxs`, `vctuxs`, `vcfsx` and `vcfux` through `execute_vx`: the vA
//! field is the scale, a saturated slot sets `VSCR[SAT]`, and SAT is
//! sticky.

use super::lane_support::pack_u32x4;
use crate::exec::test_support::exec_no_mem;
use crate::exec::ExecuteVerdict;
use crate::instruction::ops::VxOp;
use crate::instruction::PpuInstruction;
use crate::state::PpuState;
use cellgov_ps3_abi::hw::ppu::{VSCR_NJ, VSCR_SAT};

/// vD and `VSCR` after `op` with scale `uimm` on vB = `b`, from `vscr`.
fn run(op: VxOp, vscr: u32, uimm: u8, b: u128) -> (u128, u32) {
    let mut s = PpuState::new();
    s.set_vscr(vscr);
    s.set_vr(2, b);
    let insn = PpuInstruction::Vx {
        op,
        rc: false,
        vt: 3,
        va: uimm,
        vb: 2,
    };
    assert_eq!(
        exec_no_mem(&insn, &mut s),
        ExecuteVerdict::Continue,
        "{op:?}"
    );
    (s.vr[3], s.vscr())
}

/// [AltiVec-PEM p:6-65 s:6.2] vctsxs: one slot past 2^31 sets SAT; the
/// others truncate toward zero.
#[test]
fn vctsxs_saturates_one_slot_and_truncates_the_rest() {
    // 2^31, -1.5, 1.75 and -2^31.
    let b = pack_u32x4([0x4F00_0000, 0xBFC0_0000, 0x3FE0_0000, 0xCF00_0000]);
    assert_eq!(
        run(VxOp::Vctsxs, 0, 0, b),
        (
            pack_u32x4([0x7FFF_FFFF, 0xFFFF_FFFF, 1, 0x8000_0000]),
            VSCR_SAT
        )
    );
    // Scaled by 2: 1.75 x 2 = 3.5 truncates to 3, -1.5 x 2 = -3.
    let in_range = pack_u32x4([0, 0xBFC0_0000, 0x3FE0_0000, 0x8000_0000]);
    assert_eq!(
        run(VxOp::Vctsxs, 0, 1, in_range),
        (pack_u32x4([0, 0xFFFF_FFFD, 3, 0]), 0)
    );
}

/// [AltiVec-PEM p:6-66 s:6.2] vctuxs: a negative slot saturates to 0 and
/// sets SAT.
#[test]
fn vctuxs_saturates_a_negative_slot_to_zero() {
    // -1, 2^32 - 256, 3.5 and +inf.
    let b = pack_u32x4([0xBF80_0000, 0x4F7F_FFFF, 0x4060_0000, 0x7F80_0000]);
    assert_eq!(
        run(VxOp::Vctuxs, 0, 0, b),
        (pack_u32x4([0, 0xFFFF_FF00, 3, 0xFFFF_FFFF]), VSCR_SAT)
    );
    // NJ = 1 saturates the same slots and sets SAT the same way.
    assert_eq!(
        run(VxOp::Vctuxs, VSCR_NJ, 0, b),
        (
            pack_u32x4([0, 0xFFFF_FF00, 3, 0xFFFF_FFFF]),
            VSCR_SAT | VSCR_NJ
        )
    );
}

#[test]
fn an_in_range_conversion_keeps_sat_set() {
    let one = pack_u32x4([0x3F80_0000; 4]);
    for op in [VxOp::Vctsxs, VxOp::Vctuxs] {
        assert_eq!(
            run(op, VSCR_SAT | VSCR_NJ, 0, one),
            (pack_u32x4([1; 4]), VSCR_SAT | VSCR_NJ),
            "{op:?}"
        );
        assert_eq!(run(op, 0, 0, one), (pack_u32x4([1; 4]), 0), "{op:?}");
    }
}

/// [AltiVec-PEM p:3-15 s:3.2.5.4] a NaN converts to 0.
/// A denormal times at most 2^31 stays below 1, so it truncates to 0 in
/// both modes; the ppu_vfp_convert capture shows neither case sets SAT.
#[test]
fn a_nan_or_a_denormal_converts_to_zero_without_sat() {
    let b = pack_u32x4([0x7FC0_0000, 0xFF80_0001, 0x0000_0001, 0x807F_FFFF]);
    for op in [VxOp::Vctsxs, VxOp::Vctuxs] {
        for vscr in [0, VSCR_NJ] {
            assert_eq!(run(op, vscr, 31, b), (0, vscr), "{op:?} {vscr:#x}");
        }
    }
}

/// [AltiVec-PEM p:6-49 s:6.2] vcfsx and vcfux take the scale from the vA field and never touch SAT.
#[test]
fn integer_to_float_takes_the_scale_from_the_va_field() {
    let b = pack_u32x4([1, 0xFFFF_FFFF, 0x0100_0001, 0x8000_0000]);
    assert_eq!(
        run(VxOp::Vcfsx, 0, 31, b),
        (
            pack_u32x4([0x3000_0000, 0xB000_0000, 0x3C00_0000, 0xBF80_0000]),
            0
        )
    );
    assert_eq!(
        run(VxOp::Vcfux, VSCR_SAT, 0, b),
        (
            pack_u32x4([0x3F80_0000, 0x4F80_0000, 0x4B80_0000, 0x4F00_0000]),
            VSCR_SAT
        )
    );
}
