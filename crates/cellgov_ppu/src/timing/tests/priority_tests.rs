//! A thread beside an outranking sibling: it issues no more often than
//! the sibling does.

use super::{classify, PpeTiming};
use crate::instruction::PpuInstruction as I;
use crate::state::PpuState;
use cellgov_exec::PpeIssuePressure;

/// Retire `insns` from `pc` up as straight-line code, `times` times over,
/// and return the issue cycle of each op of the last pass.
fn run(timing: &mut PpeTiming, insns: &[I], times: usize) -> Vec<u64> {
    let state = PpuState::new();
    let mut cycles = Vec::new();
    for _ in 0..times {
        cycles.clear();
        for (i, insn) in insns.iter().enumerate() {
            let at = 0x1000 + 4 * i as u64;
            let (ops, n) = classify(insn, &state);
            for op in &ops[..n] {
                cycles.push(timing.issue_cycle(at, op));
                timing.retire(at, op, at + 4);
            }
        }
    }
    cycles
}

/// A sibling that outranks the receiving thread and issues once every
/// `cycles` cycles, with no stall.
fn outranking(cycles: u64) -> PpeIssuePressure {
    PpeIssuePressure {
        issue_interval: (cycles * 256) as u32,
        ..PpeIssuePressure::NONE
    }
}

fn addis() -> [I; 4] {
    let addi = |rt| I::Addi { rt, ra: rt, imm: 1 };
    [addi(17), addi(18), addi(19), addi(20)]
}

fn fadd() -> I {
    I::Fp63 {
        op: crate::instruction::ops::Fp63Op::Fadd,
        frt: 0,
        fra: 0,
        frb: 12,
        frc: 0,
        rc: false,
    }
}

fn gaps(cycles: &[u64]) -> Vec<u64> {
    cycles.windows(2).map(|w| w[1] - w[0]).collect()
}

#[test]
fn a_thread_beside_an_outranking_sibling_issues_once_each_of_its_intervals() {
    let mut beside = PpeTiming::new();
    beside.set_siblings(1, outranking(10));
    let issued = run(&mut beside, &addis(), 8);
    assert_eq!(gaps(&issued), [10; 3]);
}

#[test]
fn a_sibling_that_does_not_outrank_leaves_the_thread_its_own_rate() {
    let mut beside = PpeTiming::new();
    beside.set_siblings(1, PpeIssuePressure::NONE);
    let issued = run(&mut beside, &addis(), 8);
    assert!(gaps(&issued).iter().all(|&g| g < 10), "{issued:?}");
}

#[test]
fn a_thread_passes_its_own_mean_interval_between_issues() {
    let mut alone = PpeTiming::new();
    let issued = run(&mut alone, &[fadd(); 8], 8);
    let own = gaps(&issued);
    assert!(own.iter().all(|&g| g == own[0]), "{issued:?}");
    let interval = alone.pressure().issue_interval;
    let want = own[0] as u32 * 256;
    assert!(
        interval.abs_diff(want) <= want / 50,
        "interval {interval}, issues {issued:?}"
    );
}

#[test]
fn a_gate_a_departed_sibling_set_holds_nothing_once_another_returns() {
    let mut beside = PpeTiming::new();
    beside.set_siblings(1, outranking(40));
    let first = run(&mut beside, &addis()[..1], 1)[0];
    beside.set_siblings(0, PpeIssuePressure::NONE);
    beside.set_siblings(1, outranking(1));
    let next = run(&mut beside, &addis()[1..2], 1)[0];
    assert!(next - first < 40, "{first} then {next}");
}

#[test]
fn the_wait_an_outranking_sibling_adds_presses_no_sibling_in_turn() {
    // The higher-priority thread issues as it would alone beside the
    // gated one, so the gate is no wait of the gated thread's own.
    let mut beside = PpeTiming::new();
    beside.set_siblings(1, outranking(10));
    run(&mut beside, &addis(), 8);
    let p = beside.pressure();
    assert_eq!(
        (p.cycles, p.vsu_cycles, p.multiply_cycles),
        (0, 0, 0),
        "{p:?}"
    );
}
