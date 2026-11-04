//! `mfvscr`, `mtvscr` and the sticky SAT bit.

use crate::exec::test_support::exec_no_mem;
use crate::exec::ExecuteVerdict;
use crate::instruction::ops::VxOp;
use crate::instruction::PpuInstruction;
use crate::state::PpuState;
use cellgov_ps3_abi::hw::ppu::{PPU_THREAD_ENTRY_VSCR, VSCR_NJ, VSCR_SAT};

fn mtvscr(s: &mut PpuState, value: u128) {
    s.set_vr(30, value);
    assert_eq!(
        exec_no_mem(&PpuInstruction::Mtvscr { vb: 30 }, s),
        ExecuteVerdict::Continue
    );
}

fn mfvscr(s: &mut PpuState) -> u128 {
    s.set_vr(31, u128::MAX);
    assert_eq!(
        exec_no_mem(&PpuInstruction::Mfvscr { vt: 31 }, s),
        ExecuteVerdict::Continue
    );
    s.vr[31]
}

fn vx(s: &mut PpuState, op: VxOp, a: u128, b: u128) -> u128 {
    s.set_vr(1, a);
    s.set_vr(2, b);
    let insn = PpuInstruction::Vx {
        op,
        rc: false,
        vt: 3,
        va: 1,
        vb: 2,
    };
    assert_eq!(exec_no_mem(&insn, s), ExecuteVerdict::Continue);
    s.vr[3]
}

fn sat(s: &PpuState) -> bool {
    s.vscr() & VSCR_SAT != 0
}

#[test]
fn a_new_state_starts_at_the_thread_entry_vscr() {
    assert_eq!(PpuState::new().vscr(), PPU_THREAD_ENTRY_VSCR);
}

/// [AltiVec-PEM p:6-23 s:6.2], [AltiVec-PEM p:6-24 s:6.2] mfvscr gives 96 zero bits and the VSCR.
#[test]
fn the_vscr_round_trips_with_the_upper_96_bits_zero() {
    for value in [0, VSCR_NJ, VSCR_SAT, VSCR_NJ | VSCR_SAT] {
        let mut s = PpuState::new();
        mtvscr(&mut s, u128::from(value));
        assert_eq!(mfvscr(&mut s), u128::from(value), "VSCR {value:#x}");
    }
}

#[test]
fn mtvscr_reads_only_the_low_word_of_vb() {
    let mut s = PpuState::new();
    mtvscr(&mut s, (u128::MAX << 32) | u128::from(VSCR_SAT));
    assert_eq!(s.vscr(), VSCR_SAT);
}

#[test]
fn mtvscr_drops_the_reserved_bits() {
    let mut s = PpuState::new();
    mtvscr(&mut s, u128::from(u32::MAX));
    assert_eq!(s.vscr(), VSCR_NJ | VSCR_SAT);
}

/// [AltiVec-PEM p:4-4 s:4.2.1.1] A result equal to the bound is not saturation; SAT is sticky.
#[test]
fn vsububs_sets_sat_only_when_a_lane_clamps_and_sat_sticks() {
    let fives = u128::from_be_bytes([5; 16]);
    let mut s = PpuState::new();
    mtvscr(&mut s, 0);

    assert_eq!(vx(&mut s, VxOp::Vsububs, fives, fives), 0);
    assert!(!sat(&s), "5 - 5 reaches the bound without clamping");

    let mut one_clamps = [9u8; 16];
    one_clamps[7] = 3;
    vx(
        &mut s,
        VxOp::Vsububs,
        u128::from_be_bytes(one_clamps),
        fives,
    );
    assert!(sat(&s), "3 - 5 clamps");

    vx(&mut s, VxOp::Vsububs, fives, fives);
    assert!(sat(&s), "a later non-clamping vsububs leaves SAT set");

    mtvscr(&mut s, 0);
    assert!(!sat(&s), "mtvscr 0 clears SAT");
}

#[test]
fn the_modulo_adds_never_set_sat_when_they_wrap() {
    let mut s = PpuState::new();
    mtvscr(&mut s, 0);
    // All ones plus 1 in the last element of each doubleword wraps
    // that element to 0.
    for (op, lane) in [
        (VxOp::Vaddubm, 0xFFu128),
        (VxOp::Vadduhm, 0xFFFF),
        (VxOp::Vadduwm, 0xFFFF_FFFF),
    ] {
        let wrapped = !(lane | (lane << 64));
        assert_eq!(vx(&mut s, op, u128::MAX, 1 | (1 << 64)), wrapped, "{op:?}");
        assert!(!sat(&s), "{op:?}");
    }
}
