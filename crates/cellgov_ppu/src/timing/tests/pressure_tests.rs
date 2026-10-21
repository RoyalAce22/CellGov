use super::{classify, PpeTiming};
use crate::instruction::PpuInstruction as I;
use crate::state::PpuState;
use cellgov_exec::PpeIssuePressure;
use cellgov_ps3_abi::hw::ppu_issue::{
    PPU_MULLI_LATENCY_CYCLES, PPU_SMT_MICROCODE_STALL_CYCLES, PPU_SMT_SHARED_STALL_CYCLES,
};

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

fn pressure(
    cycles: u64,
    vsu_cycles: u64,
    multiply_cycles: u64,
    multiply_hold_share: u32,
) -> PpeIssuePressure {
    PpeIssuePressure {
        cycles: cycles as u32,
        vsu_cycles: vsu_cycles as u32,
        multiply_cycles: multiply_cycles as u32,
        multiply_hold_share,
    }
}

/// A sibling of divides: the shared stall, and a multiply's whole hold
/// again.
fn divide_sibling() -> PpeIssuePressure {
    let all = PPU_SMT_SHARED_STALL_CYCLES;
    pressure(all, all, all, 256)
}

/// A sibling of microcoded loads: the microcode stall, half of it for a
/// VSU instruction, the divide cap for a multiply, and no multiply hold.
fn microcode_sibling() -> PpeIssuePressure {
    let all = PPU_SMT_MICROCODE_STALL_CYCLES;
    pressure(all, all / 2, PPU_SMT_SHARED_STALL_CYCLES, 0)
}

fn fadd(frt: u8, frb: u8) -> I {
    I::Fp63 {
        op: crate::instruction::ops::Fp63Op::Fadd,
        frt,
        fra: frt,
        frb,
        frc: 0,
        rc: false,
    }
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

fn mulli() -> I {
    I::Mulli {
        rt: 9,
        ra: 9,
        imm: 1,
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
fn a_thread_of_dependent_divides_presses_a_sibling_by_the_shared_stalls() {
    let mut timing = PpeTiming::new();
    run(&mut timing, &state(), &[divd(); 8], 4);
    // Every cycle of the run is a divide's, so the whole hold repeats.
    assert_eq!(timing.pressure(), divide_sibling());
}

#[test]
fn a_thread_of_microcoded_loads_presses_a_sibling_s_multiplies_as_its_other_instructions() {
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
    assert_eq!(timing.pressure(), microcode_sibling());
}

#[test]
fn a_thread_of_mfcr_presses_by_the_microcode_stall_and_repeats_no_multiply_hold() {
    let mut timing = PpeTiming::new();
    run(&mut timing, &state(), &[I::Mfcr { rt: 9 }; 8], 4);
    assert_eq!(timing.pressure(), microcode_sibling());
}

#[test]
fn a_divide_sibling_presses_harder_than_a_microcode_one_except_on_a_multiply() {
    let lha = |rt, imm| I::Lha { rt, ra: 29, imm };
    let mut s = state();
    s.set_gpr(29, 0x2000);
    let mut divides = PpeTiming::new();
    run(&mut divides, &state(), &[divd(); 8], 4);
    let mut microcode = PpeTiming::new();
    run(
        &mut microcode,
        &s,
        &[lha(17, 0), lha(18, 2), lha(19, 4), lha(20, 6)],
        8,
    );
    let (d, m) = (divides.pressure(), microcode.pressure());
    assert!(m.cycles < d.cycles, "{m:?} {d:?}");
    assert_eq!(m.multiply_cycles, d.multiply_cycles, "{m:?} {d:?}");
}

#[test]
fn a_thread_of_dependent_multiplies_presses_by_a_share_of_its_wait() {
    // A dependent mulld waits just under 13 cycles past its floor: half
    // rounds to 6, half again to 3 for a VSU instruction, and two thirds
    // to 9.
    let mut timing = PpeTiming::new();
    run(&mut timing, &state(), &[mulld(); 8], 4);
    assert_eq!(timing.pressure(), pressure(6, 3, 9, 0));
}

#[test]
fn a_thread_of_dependent_fadds_presses_by_half_its_wait_in_the_vsu_queue() {
    // Each fadd waits 9 cycles past its floor in the VSU queue, which
    // counts half: 4.5. Half of that is 2, and half again 1.
    let mut timing = PpeTiming::new();
    run(&mut timing, &state(), &[fadd(0, 12); 8], 4);
    assert_eq!(timing.pressure(), pressure(2, 1, 3, 0));
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
    assert_eq!(timing.pressure(), PpeIssuePressure::NONE);
}

#[test]
fn an_instruction_beside_a_pressing_sibling_issues_the_pressure_after_it_is_ready() {
    // The second add of an aligned pair reads the first: it opens no
    // dispatch group, so no hold, and waits the add's 2 cycles and then
    // the sibling's stall.
    let mut timing = PpeTiming::new();
    timing.set_siblings(1, divide_sibling());
    let cycles = run(&mut timing, &state(), &[add(), add()], 1);
    assert_eq!(cycles[1] - cycles[0], 2 + PPU_SMT_SHARED_STALL_CYCLES);

    let mut alone = PpeTiming::new();
    alone.set_siblings(1, PpeIssuePressure::NONE);
    let cycles = run(&mut alone, &state(), &[add(), add()], 1);
    assert_eq!(cycles[1] - cycles[0], 2);
}

#[test]
fn a_multiply_beside_a_divide_sibling_waits_the_shared_stall_and_its_hold_again() {
    let mut timing = PpeTiming::new();
    timing.set_siblings(1, divide_sibling());
    let cycles = run(&mut timing, &state(), &[mulli(), mulli()], 1);
    let hold = PPU_MULLI_LATENCY_CYCLES;
    assert_eq!(
        cycles[1] - cycles[0],
        PPU_MULLI_LATENCY_CYCLES + PPU_SMT_SHARED_STALL_CYCLES + hold
    );

    // Beside a sibling that divides half its cycles, a mullw repeats half
    // its 11-cycle hold, rounded: 6.
    let mullw = I::Mullw {
        rt: 9,
        ra: 9,
        rb: 15,
        oe: false,
        rc: false,
    };
    let mut half = PpeTiming::new();
    half.set_siblings(1, pressure(13, 13, 13, 128));
    let cycles = run(&mut half, &state(), &[mullw, mullw], 1);
    assert_eq!(cycles[1] - cycles[0], 11 + 13 + 6);

    // Beside a sibling whose stalls are operand waits, the multiply
    // waits its share of them and no hold.
    let mut beside_waits = PpeTiming::new();
    beside_waits.set_siblings(1, pressure(6, 3, 9, 0));
    let cycles = run(&mut beside_waits, &state(), &[mulli(), mulli()], 1);
    assert_eq!(cycles[1] - cycles[0], PPU_MULLI_LATENCY_CYCLES + 9);
}

#[test]
fn the_cycles_a_sibling_adds_stay_out_of_the_thread_s_own_pressure() {
    // Beside a divide sibling each add waits 13 cycles more past its
    // operand, and a group opener no hold on top: the stall holds it
    // already. None of that is the thread's own wait.
    let chain = [add(); 8];
    let mut beside = PpeTiming::new();
    beside.set_siblings(1, divide_sibling());
    let cycles = run(&mut beside, &state(), &chain, 8);
    let gaps: Vec<u64> = cycles.windows(2).map(|w| w[1] - w[0]).collect();
    assert_eq!(gaps, [2 + PPU_SMT_SHARED_STALL_CYCLES; 7]);
    let mut alone = PpeTiming::new();
    run(&mut alone, &state(), &chain, 8);
    assert_eq!(beside.pressure(), alone.pressure());
}

#[test]
fn the_younger_of_a_pair_waits_the_sibling_s_stall_too() {
    // An addi and an independent fadd pair: alone the fadd issues in the
    // addi's cycle. Beside a divide sibling it waits the stall after it.
    let pair = [
        I::Addi {
            rt: 17,
            ra: 17,
            imm: 1,
        },
        fadd(9, 12),
    ];
    let mut alone = PpeTiming::new();
    let cycles = run(&mut alone, &state(), &pair, 1);
    assert_eq!(cycles[1], cycles[0]);
    let mut beside = PpeTiming::new();
    beside.set_siblings(1, divide_sibling());
    let cycles = run(&mut beside, &state(), &pair, 1);
    assert_eq!(cycles[1] - cycles[0], PPU_SMT_SHARED_STALL_CYCLES);
}

#[test]
fn a_sibling_that_presses_by_one_costs_no_less_than_one_that_presses_by_none() {
    // A microcoded lha leaves no group open, so each one opens its own and
    // waits the hold. The stall of one cycle covers that much of the hold
    // and no more.
    let lha = |rt, imm| I::Lha { rt, ra: 29, imm };
    let chain: Vec<I> = (0..16)
        .map(|i| lha(17 + i % 4, 2 * (i % 4) as i16))
        .collect();
    let mut s = state();
    s.set_gpr(29, 0x2000);
    let last = |pressure| {
        let mut timing = PpeTiming::new();
        timing.set_siblings(1, pressure);
        let cycles = run(&mut timing, &s, &chain, 2);
        cycles[15] - cycles[0]
    };
    let none = last(PpeIssuePressure::NONE);
    let one = last(pressure(1, 1, 1, 0));
    assert!(one >= none, "pressure 1: {one}, pressure 0: {none}");
}

#[test]
fn a_stall_delays_the_results_still_pending_when_it_began() {
    // A dependent fadd, then an independent addi. Alone the addi hides in
    // the fadd's 10 cycles. Beside a divide sibling the next fadd waits
    // its latency and both stalls.
    let addi = I::Addi {
        rt: 17,
        ra: 17,
        imm: 1,
    };
    let chain = [fadd(0, 12), addi, fadd(0, 12), addi];
    let mut alone = PpeTiming::new();
    let cycles = run(&mut alone, &state(), &chain, 4);
    assert_eq!(cycles[2] - cycles[0], 10);
    let mut beside = PpeTiming::new();
    beside.set_siblings(1, divide_sibling());
    let cycles = run(&mut beside, &state(), &chain, 4);
    assert_eq!(cycles[2] - cycles[0], 10 + 2 * PPU_SMT_SHARED_STALL_CYCLES);
}

#[test]
fn beside_microcode_a_vsu_instruction_waits_half_the_stall() {
    let chain = [fadd(0, 12), fadd(0, 12)];
    let mut beside = PpeTiming::new();
    beside.set_siblings(1, microcode_sibling());
    let cycles = run(&mut beside, &state(), &chain, 1);
    assert_eq!(
        cycles[1] - cycles[0],
        10 + PPU_SMT_MICROCODE_STALL_CYCLES / 2
    );
    let mut divides = PpeTiming::new();
    divides.set_siblings(1, divide_sibling());
    let cycles = run(&mut divides, &state(), &chain, 1);
    assert_eq!(cycles[1] - cycles[0], 10 + PPU_SMT_SHARED_STALL_CYCLES);
}

#[test]
fn a_hold_the_thread_s_own_wait_covers_stays_in_its_pressure() {
    // Each dependent fadd waits 9 cycles past its floor, which covers a
    // group opener's whole hold: the wait is the thread's own.
    let chain = [fadd(0, 12); 8];
    let mut beside = PpeTiming::new();
    beside.set_siblings(1, PpeIssuePressure::NONE);
    run(&mut beside, &state(), &chain, 8);
    let mut alone = PpeTiming::new();
    run(&mut alone, &state(), &chain, 8);
    assert_ne!(alone.pressure(), PpeIssuePressure::NONE);
    assert_eq!(beside.pressure(), alone.pressure());
}

#[test]
fn a_thread_whose_sibling_goes_idle_issues_without_its_stall() {
    let mut timing = PpeTiming::new();
    timing.set_siblings(1, divide_sibling());
    timing.set_siblings(0, divide_sibling());
    let cycles = run(&mut timing, &state(), &[add(), add()], 1);
    assert_eq!(cycles[1] - cycles[0], 2);
}
