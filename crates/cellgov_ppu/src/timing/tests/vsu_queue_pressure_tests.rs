use super::{classify, PpeTiming};
use crate::instruction::ops::VxOp;
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
            let issued = timing.retire_instruction(at, &ops[..n], at + 4 * n.max(1) as u64);
            cycles.extend_from_slice(&issued[..n]);
        }
    }
    cycles
}

/// `vaddubm vt, va, vb`.
fn vadd(vt: u8, va: u8, vb: u8) -> I {
    I::Vx {
        op: VxOp::Vaddubm,
        rc: false,
        vt,
        va,
        vb,
    }
}

const MTVSCR: I = I::Mtvscr { vb: 0 };

/// A sibling that presses by nothing but `queue` and `fence` cycles.
fn queue_sibling(queue: u32, fence: u32) -> PpeIssuePressure {
    PpeIssuePressure {
        queue_cycles: queue,
        fence_cycles: fence,
        ..PpeIssuePressure::NONE
    }
}

/// The cycles from the first op to the last of `insns`, beside `sibling`.
fn span(insns: &[I], sibling: PpeIssuePressure) -> u64 {
    let mut timing = PpeTiming::new();
    timing.set_siblings(1, sibling);
    let cycles = run(&mut timing, insns, 1);
    cycles[cycles.len() - 1] - cycles[0]
}

#[test]
fn a_thread_whose_vector_work_waits_at_its_fence_holds_the_queue_head_by_that_wait() {
    let mut timing = PpeTiming::new();
    run(
        &mut timing,
        &[MTVSCR, vadd(1, 20, 21), vadd(2, 20, 21), vadd(3, 20, 21)],
        8,
    );
    let p = timing.pressure();
    assert!(p.fence_cycles > 0, "{p:?}");
    assert_eq!(p.queue_cycles, p.fence_cycles, "{p:?}");
}

#[test]
fn a_held_vector_op_beside_the_queue_head_of_a_sibling_runs_that_much_later() {
    // The second vaddubm reads the first, which the fence holds: it issues
    // once the first's result is ready.
    let chain = [MTVSCR, vadd(1, 20, 21), vadd(2, 1, 1)];
    let alone = span(&chain, PpeIssuePressure::NONE);
    assert_eq!(span(&chain, queue_sibling(9, 0)), alone + 9);
}

#[test]
fn a_drain_on_a_thread_whose_queue_never_held_an_op_issues_at_once() {
    let mut timing = PpeTiming::new();
    assert_eq!(run(&mut timing, &[I::Sync { l: 0 }], 1), [0]);
}

#[test]
fn an_unheld_vector_op_waits_only_the_sibling_s_fence_part_of_the_queue_head() {
    let chain = [vadd(1, 20, 21), vadd(2, 1, 1)];
    let alone = span(&chain, PpeIssuePressure::NONE);
    assert_eq!(span(&chain, queue_sibling(9, 0)), alone);
    assert_eq!(span(&chain, queue_sibling(9, 5)), alone + 5);
}
