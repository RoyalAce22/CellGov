use super::{PRESSURE_WINDOW_INSTRUCTIONS, STALL_WINDOW_CYCLES};
use crate::instruction::ops::Fp63Op;
use crate::instruction::PpuInstruction as I;
use crate::state::PpuState;
use crate::timing::{classify, PpeTiming};
use cellgov_ps3_abi::lv2::syscall as sc;

/// Retire `insn` at `pc` as straight-line code, and return the issue
/// cycle of its first op.
fn retire(timing: &mut PpeTiming, state: &PpuState, pc: u64, insn: &I) -> u64 {
    let (ops, n) = classify(insn, state);
    timing.retire_instruction(pc, &ops[..n], pc + 4 * n.max(1) as u64)[0]
}

/// `fdiv f0,f0,f12`: each reads the one before.
fn fdiv() -> I {
    I::Fp63 {
        op: Fp63Op::Fdiv,
        frt: 0,
        fra: 0,
        frb: 12,
        frc: 0,
        rc: false,
    }
}

/// A fresh thread after 16 dependent `fdiv` from 0x1000, and the address
/// after them.
fn after_divides(state: &PpuState) -> (PpeTiming, u64) {
    let mut timing = PpeTiming::new();
    let mut pc = 0x1000;
    for _ in 0..16 {
        retire(&mut timing, state, pc, &fdiv());
        pc += 4;
    }
    assert_ne!(timing.core.divide_interval, 0);
    (timing, pc)
}

#[test]
fn a_divide_at_cycle_zero_is_the_first_sample_of_the_divide_interval() {
    let state = PpuState::new();
    let mut timing = PpeTiming::new();
    let first = retire(&mut timing, &state, 0x1000, &fdiv());
    assert_eq!(first, 0);
    let second = retire(&mut timing, &state, 0x1004, &fdiv());
    let n = u64::from(PRESSURE_WINDOW_INSTRUCTIONS);
    assert_eq!(
        u64::from(timing.core.divide_interval),
        (second - first) * 256 / n
    );
}

#[test]
fn an_lv2_call_between_two_divides_adds_no_sample_to_the_divide_interval() {
    let mut state = PpuState::new();
    state.set_gpr(11, sc::TIME_GET_TIMEBASE_FREQUENCY);
    let (mut timing, mut pc) = after_divides(&state);
    let call = |timing: &mut PpeTiming, pc: &mut u64| {
        retire(timing, &state, *pc, &I::Sc { lev: 0 });
        timing.charge_syscall();
        *pc += 4;
    };
    call(&mut timing, &mut pc);
    let before = retire(&mut timing, &state, pc, &fdiv());
    pc += 4;
    let want = timing.core.divide_interval;
    call(&mut timing, &mut pc);
    let after = retire(&mut timing, &state, pc, &fdiv());
    assert!(
        after - before <= STALL_WINDOW_CYCLES,
        "the gap of {} cycles is one the stall window bounds",
        after - before
    );
    assert_eq!(timing.core.divide_interval, want);
}

#[test]
fn back_to_back_vsu_divides_open_the_window() {
    let state = PpuState::new();
    let (mut timing, mut pc) = after_divides(&state);
    for _ in 0..64 {
        retire(&mut timing, &state, pc, &fdiv());
        pc += 4;
    }
    assert_ne!(timing.pressure().window_cycles, 0);
}

#[test]
fn back_to_back_fixed_point_divides_after_vsu_divides_open_no_window() {
    let mut state = PpuState::new();
    state.set_gpr(15, 0x7fff_0000);
    state.set_gpr(16, 1);
    let (mut timing, mut pc) = after_divides(&state);
    let divd = I::Divd {
        rt: 15,
        ra: 15,
        rb: 16,
        oe: false,
        rc: false,
    };
    for _ in 0..64 {
        retire(&mut timing, &state, pc, &divd);
        pc += 4;
    }
    assert!(timing.core.stall_share >= super::WINDOW_STALL_SHARE);
    assert_eq!(timing.pressure().window_cycles, 0);
}

#[test]
fn a_divide_after_more_than_the_stall_window_of_other_work_adds_no_sample() {
    let state = PpuState::new();
    let (mut timing, mut pc) = after_divides(&state);
    let want = timing.core.divide_interval;
    let from = timing.now();
    let addi = |rt| I::Addi { rt, ra: rt, imm: 1 };
    let most = 16 * STALL_WINDOW_CYCLES;
    for i in 0..most {
        if timing.now() - from > 2 * STALL_WINDOW_CYCLES {
            break;
        }
        retire(&mut timing, &state, pc, &addi(17 + (i % 4) as u8));
        pc += 4;
    }
    assert!(
        timing.now() - from > 2 * STALL_WINDOW_CYCLES,
        "the addi did not pass two stall windows"
    );
    retire(&mut timing, &state, pc, &fdiv());
    assert_eq!(timing.core.divide_interval, want);
}
