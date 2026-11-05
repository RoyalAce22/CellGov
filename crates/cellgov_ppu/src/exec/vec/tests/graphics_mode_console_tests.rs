//! The `ppu_vscr_mode` capture shows the mode of the vector unit: a game
//! process runs it in IEEE mode.
//!
//! [CBE-Handbook p:744 s:A.3.3] The graphics rounding mode rounds toward
//! zero and produces no infinity and no NaN. The capture's words 5 to 7
//! (NJ = 0) and 10 to 12 (NJ = 1) show a result rounded up, a NaN and an
//! infinity. Thus the process runs with the graphics rounding mode off.

use super::lane_support::pack_u32x4;
use super::vscr_console_tests::console_words;
use crate::exec::test_support::exec_no_mem;
use crate::instruction::ops::{VaOp, VxOp};
use crate::instruction::PpuInstruction;
use crate::state::PpuState;
use cellgov_ps3_abi::hw::ppu::VSCR_NJ;

/// The first payload word of each NJ mode's five results.
const SETS: [(usize, bool); 2] = [(5, false), (10, true)];

fn is_nan(word: u32) -> bool {
    word & 0x7F80_0000 == 0x7F80_0000 && word & 0x007F_FFFF != 0
}

fn is_infinity(word: u32) -> bool {
    word & 0x7FFF_FFFF == 0x7F80_0000
}

fn vaddfp(vscr: u32, a: u32, b: u32) -> u32 {
    let mut s = PpuState::new();
    s.set_vscr(vscr);
    s.set_vr(1, pack_u32x4([a; 4]));
    s.set_vr(2, pack_u32x4([b; 4]));
    let insn = PpuInstruction::Vx {
        op: VxOp::Vaddfp,
        rc: false,
        vt: 3,
        va: 1,
        vb: 2,
    };
    exec_no_mem(&insn, &mut s);
    (s.vr[3] >> 96) as u32
}

/// `a * c + b`, in the probe's operand order.
fn vmaddfp(vscr: u32, a: u32, c: u32, b: u32) -> u32 {
    let mut s = PpuState::new();
    s.set_vscr(vscr);
    s.set_vr(1, pack_u32x4([a; 4]));
    s.set_vr(2, pack_u32x4([b; 4]));
    s.set_vr(4, pack_u32x4([c; 4]));
    let insn = PpuInstruction::Va {
        op: VaOp::Vmaddfp,
        vt: 3,
        va: 1,
        vb: 2,
        vc: 4,
    };
    exec_no_mem(&insn, &mut s);
    (s.vr[3] >> 96) as u32
}

#[test]
fn cellgov_and_the_console_round_to_nearest_and_give_nans_and_infinities_in_both_nj_modes() {
    let console = console_words("ppu_vscr_mode", 19);
    for (base, nj) in SETS {
        let vscr = if nj { VSCR_NJ } else { 0 };
        // vaddfp(1.0, 2^-24 * 1.5): round toward zero would give 1.0.
        assert_ne!(console[base], 0x3F80_0000, "nj={nj}: rounded toward zero");
        assert!(is_nan(console[base + 1]), "nj={nj}: Inf - Inf gave no NaN");
        assert!(
            is_infinity(console[base + 2]),
            "nj={nj}: an overflowing vmaddfp gave no infinity"
        );

        let cellgov = [
            vaddfp(vscr, 0x3F80_0000, 0x33C0_0000),
            vaddfp(vscr, 0x7F80_0000, 0xFF80_0000),
            vmaddfp(vscr, 0x7F7F_FFFF, 0x4000_0000, 0x8000_0000),
        ];
        assert_eq!(cellgov, console[base..base + 3], "nj={nj}");
    }
}

#[test]
fn vmaddfp_overflow_and_vcfsx_give_the_words_the_console_wrote() {
    let console = console_words("ppu_vscr_mode", 19);
    for (base, nj) in SETS {
        let vscr = if nj { VSCR_NJ } else { 0 };

        let mut s = PpuState::new();
        s.set_vscr(vscr);
        s.set_vr(1, pack_u32x4([0x7F7F_FFFF; 4]));
        s.set_vr(4, pack_u32x4([0x4000_0000; 4]));
        s.set_vr(2, pack_u32x4([0x8000_0000; 4]));
        let madd = PpuInstruction::Va {
            op: VaOp::Vmaddfp,
            vt: 3,
            va: 1,
            vb: 2,
            vc: 4,
        };
        exec_no_mem(&madd, &mut s);
        assert_eq!(
            s.vr[3],
            pack_u32x4([console[base + 2]; 4]),
            "nj={nj} vmaddfp"
        );

        let mut s = PpuState::new();
        s.set_vscr(vscr);
        s.set_vr(2, pack_u32x4([0x7FFF_FFFF; 4]));
        let cfsx = PpuInstruction::Vx {
            op: VxOp::Vcfsx,
            rc: false,
            vt: 3,
            va: 0,
            vb: 2,
        };
        exec_no_mem(&cfsx, &mut s);
        assert_eq!(s.vr[3], pack_u32x4([console[base + 4]; 4]), "nj={nj} vcfsx");
    }
}
