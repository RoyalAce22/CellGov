use super::{classify, PpeTiming};
use crate::instruction::PpuInstruction as I;
use crate::state::PpuState;
use cellgov_ps3_abi::hw::ppu_issue::{PPU_SMT_SHARED_STALL_CYCLES, PPU_SMT_SIBLING_HOLD_CYCLES};

/// Retire `insns` from `pc` up as straight-line code, `times` times over,
/// and return the issue cycle of each op of the last pass.
fn run(timing: &mut PpeTiming, state: &PpuState, insns: &[I], times: usize) -> Vec<u64> {
    let mut cycles = Vec::new();
    for _ in 0..times {
        cycles.clear();
        for (i, insn) in insns.iter().enumerate() {
            let at = 0x1000 + 4 * i as u64;
            let (ops, n) = classify(insn, state);
            for op in &ops[..n] {
                cycles.push(timing.issue_cycle(at, op));
                timing.retire(at, op, at + 4);
            }
        }
    }
    cycles
}

fn divd() -> I {
    I::Divd {
        rt: 15,
        ra: 15,
        rb: 16,
        oe: false,
        rc: false,
    }
}

fn mulld() -> I {
    I::Mulld {
        rt: 15,
        ra: 15,
        rb: 16,
        oe: false,
        rc: false,
    }
}

fn add() -> I {
    I::Add {
        rt: 9,
        ra: 9,
        rb: 15,
        oe: false,
        rc: false,
    }
}

/// The probe's dividend and divisor.
fn state() -> PpuState {
    let mut s = PpuState::new();
    s.set_gpr(15, 0x7fff_0000);
    s.set_gpr(16, 1);
    s
}

#[test]
fn a_thread_of_dependent_divides_presses_a_sibling_by_the_shared_stall() {
    let mut timing = PpeTiming::new();
    run(&mut timing, &state(), &[divd(); 8], 4);
    assert_eq!(u64::from(timing.pressure()), PPU_SMT_SHARED_STALL_CYCLES);
}

#[test]
fn a_thread_of_microcoded_loads_presses_a_sibling_by_the_shared_stall() {
    let lha = |rt, imm| I::Lha { rt, ra: 29, imm };
    let mut s = state();
    s.set_gpr(29, 0x2000);
    let mut timing = PpeTiming::new();
    run(
        &mut timing,
        &s,
        &[lha(17, 0), lha(18, 2), lha(19, 4), lha(20, 6)],
        8,
    );
    assert_eq!(u64::from(timing.pressure()), PPU_SMT_SHARED_STALL_CYCLES);
}

#[test]
fn a_thread_of_dependent_multiplies_presses_a_sibling_by_two_fifths_of_its_wait() {
    // A dependent mulld waits 13 cycles past its floor; two fifths of 13
    // rounds to 5.
    let mut timing = PpeTiming::new();
    run(&mut timing, &state(), &[mulld(); 8], 4);
    assert_eq!(timing.pressure(), 5);
}

#[test]
fn a_thread_of_independent_adds_presses_a_sibling_by_nothing() {
    let addi = |rt| I::Addi { rt, ra: rt, imm: 1 };
    let mut timing = PpeTiming::new();
    run(
        &mut timing,
        &state(),
        &[addi(17), addi(18), addi(19), addi(20)],
        8,
    );
    assert_eq!(timing.pressure(), 0);
}

#[test]
fn an_instruction_beside_a_pressing_sibling_issues_the_pressure_after_it_is_ready() {
    // The second add of an aligned pair reads the first: it opens no
    // dispatch group, so no hold, and waits the add's 2 cycles and then
    // the sibling's stall.
    let mut timing = PpeTiming::new();
    timing.set_siblings(1, 13);
    let cycles = run(&mut timing, &state(), &[add(), add()], 1);
    assert_eq!(cycles[1] - cycles[0], 2 + 13);

    let mut alone = PpeTiming::new();
    alone.set_siblings(1, 0);
    let cycles = run(&mut alone, &state(), &[add(), add()], 1);
    assert_eq!(cycles[1] - cycles[0], 2);
}

#[test]
fn the_cycles_a_sibling_adds_stay_out_of_the_thread_s_own_pressure() {
    // Beside a divide sibling each add waits 13 cycles more past its
    // operand, and a group opener waits out the hold, one cycle of which
    // the add's latency covers. None of that is the thread's own wait.
    let chain = [add(); 8];
    let mut beside = PpeTiming::new();
    beside.set_siblings(1, 13);
    let cycles = run(&mut beside, &state(), &chain, 8);
    let gaps: Vec<u64> = cycles.windows(2).map(|w| w[1] - w[0]).collect();
    let opener = PPU_SMT_SIBLING_HOLD_CYCLES - 1 + 2 + 13;
    assert_eq!(gaps, [15, opener, 15, opener, 15, opener, 15]);
    let mut alone = PpeTiming::new();
    run(&mut alone, &state(), &chain, 8);
    assert_eq!(beside.pressure(), alone.pressure());
}

#[test]
fn a_hold_the_thread_s_own_wait_covers_stays_in_its_pressure() {
    // Each dependent fadd waits 9 cycles past its floor, which covers a
    // group opener's whole hold: the wait is the thread's own.
    let fadd = I::Fp63 {
        op: crate::instruction::ops::Fp63Op::Fadd,
        frt: 0,
        fra: 0,
        frb: 12,
        frc: 0,
        rc: false,
    };
    let chain = [fadd; 8];
    let mut beside = PpeTiming::new();
    beside.set_siblings(1, 0);
    run(&mut beside, &state(), &chain, 8);
    let mut alone = PpeTiming::new();
    run(&mut alone, &state(), &chain, 8);
    assert_ne!(alone.pressure(), 0);
    assert_eq!(beside.pressure(), alone.pressure());
}

#[test]
fn a_thread_whose_sibling_goes_idle_issues_without_its_stall() {
    let mut timing = PpeTiming::new();
    timing.set_siblings(1, 13);
    timing.set_siblings(0, 13);
    let cycles = run(&mut timing, &state(), &[add(), add()], 1);
    assert_eq!(cycles[1] - cycles[0], 2);
}
