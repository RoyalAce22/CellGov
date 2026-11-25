use crate::instruction::ops::Fp63Op;
use crate::instruction::PpuInstruction as I;
use crate::state::PpuState;
use crate::timing::{classify, PpeTiming};
use cellgov_ps3_abi::hw::ppu_issue as k;

/// The issue cycle of each op of `insns`, retired once as straight-line
/// code from a fresh model.
fn issued(insns: &[I]) -> Vec<u64> {
    let state = PpuState::new();
    let mut timing = PpeTiming::new();
    let mut cycles = Vec::new();
    for (i, insn) in insns.iter().enumerate() {
        let at = 0x1000 + 4 * i as u64;
        let (ops, n) = classify(insn, &state);
        let issued = timing.retire_instruction(at, &ops[..n], at + 4 * n.max(1) as u64);
        cycles.extend_from_slice(&issued[..n]);
    }
    cycles
}

fn fadd(rc: bool) -> I {
    I::Fp63 {
        op: Fp63Op::Fadd,
        frt: 1,
        fra: 2,
        frb: 3,
        frc: 0,
        rc,
    }
}

const MFFS: I = I::Fp63 {
    op: Fp63Op::Mffs,
    frt: 4,
    fra: 0,
    frb: 0,
    frc: 0,
    rc: false,
};

/// The cycles from an `fadd` or `fadd.` to a younger `mffs`.
fn mffs_wait(rc: bool) -> u64 {
    let cycles = issued(&[fadd(rc), MFFS]);
    cycles[1] - cycles[0]
}

#[test]
fn an_mffs_waits_for_the_cr_field_of_an_older_recording_fadd() {
    // [CBE-Handbook p:722 s:A.1.2] a recording floating-point op writes
    // its CR field cycles after its FPR, and an `mffs` waits for every
    // older vector-scalar op to complete.
    assert!(
        mffs_wait(true) >= k::PPU_FP_LATENCY_CYCLES + k::PPU_FP_RECORD_EXTRA_CYCLES,
        "the mffs issues {} cycles after the fadd.",
        mffs_wait(true),
    );
    assert_eq!(
        mffs_wait(true) - mffs_wait(false),
        k::PPU_FP_RECORD_EXTRA_CYCLES,
    );
}
