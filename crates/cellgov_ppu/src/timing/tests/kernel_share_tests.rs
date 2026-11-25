use super::KERNEL_WINDOW_CYCLES;
use crate::instruction::PpuInstruction as I;
use crate::state::PpuState;
use crate::timing::{classify, PpeTiming};
use cellgov_ps3_abi::hw::ppu_issue as k;
use cellgov_ps3_abi::lv2::syscall as sc;

/// Retire `insn` at `pc` as straight-line code.
fn retire(timing: &mut PpeTiming, state: &PpuState, pc: u64, insn: &I) {
    let (ops, n) = classify(insn, state);
    timing.retire_instruction(pc, &ops[..n], pc + 4 * n.max(1) as u64);
}

/// A fresh thread with no sibling that made one yield and charged its
/// round trip.
fn after_one_yield() -> PpeTiming {
    let mut state = PpuState::new();
    state.set_gpr(11, sc::PPU_THREAD_YIELD);
    let mut timing = PpeTiming::new();
    retire(&mut timing, &state, 0x1000, &I::Sc { lev: 0 });
    timing.charge_syscall();
    timing
}

#[test]
fn one_yield_reports_its_round_trip_s_share_of_the_window_to_the_nearest_256th() {
    let trip = k::ppu_syscall_round_trip_cycles(sc::PPU_THREAD_YIELD) as f64;
    let want = (256.0 * trip / KERNEL_WINDOW_CYCLES as f64).round() as u32;
    assert_eq!(after_one_yield().pressure().kernel_share, want);
}

#[test]
fn the_kernel_share_falls_below_half_but_not_to_zero_over_one_window_of_user_code() {
    let mut timing = after_one_yield();
    let start = timing.pressure().kernel_share;
    let from = timing.now();
    let state = PpuState::new();
    let addi = |rt| I::Addi { rt, ra: rt, imm: 1 };
    // Independent `addi` issue at most two a cycle.
    let most = 4 * KERNEL_WINDOW_CYCLES;
    let mut retired = 0;
    for i in 0..most {
        if timing.now() - from >= KERNEL_WINDOW_CYCLES {
            break;
        }
        retire(
            &mut timing,
            &state,
            0x2000 + 4 * i,
            &addi(17 + (i % 4) as u8),
        );
        retired += 1;
    }
    assert!(retired < most, "the thread's cycle did not pass one window");
    let after = timing.pressure().kernel_share;
    assert!(
        0 < after && 2 * after < start,
        "{start} of 256 after the yield, {after} after one window of addi",
    );
}
