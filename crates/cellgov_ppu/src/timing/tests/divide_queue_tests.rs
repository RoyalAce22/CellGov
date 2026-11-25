use super::{classify, PpeTiming};
use crate::instruction::ops::Fp63Op;
use crate::instruction::PpuInstruction as I;
use crate::state::PpuState;
use cellgov_exec::PpeIssuePressure;
use cellgov_ps3_abi::hw::ppu_issue::{
    PPU_FDIV_CYCLES, PPU_FP_LATENCY_CYCLES, PPU_FX_LATENCY_CYCLES, PPU_SMT_SHARED_STALL_CYCLES,
    PPU_VIQ_HELD_OPS,
};

/// Retire `insns` from `pc` up as straight-line code, `times` times over,
/// and return the issue cycle of each op of the last pass.
fn run(timing: &mut PpeTiming, insns: &[I], times: usize) -> Vec<u64> {
    let mut state = PpuState::new();
    // The dividend and divisor of [`divd`].
    state.set_gpr(15, 0x7fff_0000);
    state.set_gpr(16, 1);
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

/// `fdiv frt,f12,f13`: reads no older divide's result.
fn fdiv_to(frt: u8) -> I {
    I::Fp63 {
        op: Fp63Op::Fdiv,
        frt,
        fra: 12,
        frb: 13,
        frc: 0,
        rc: false,
    }
}

/// `fadd f1,f1,f2`: each reads the one before.
fn fadd() -> I {
    I::Fp63 {
        op: Fp63Op::Fadd,
        frt: 1,
        fra: 1,
        frb: 2,
        frc: 0,
        rc: false,
    }
}

/// `addi r3,r3,1`: each reads the one before.
fn addi_r3() -> I {
    I::Addi {
        rt: 3,
        ra: 3,
        imm: 1,
    }
}

/// `divd r15,r15,r16`: a divide on the fixed-point unit.
fn divd() -> I {
    I::Divd {
        rt: 15,
        ra: 15,
        rb: 16,
        oe: false,
        rc: false,
    }
}

/// Assert that of `cycles`, a run of divides, the queue holds
/// [`PPU_VIQ_HELD_OPS`] behind the first, and the next waits for room.
fn assert_queue_holds(cycles: &[u64], what: &str) {
    assert_eq!(cycles.len(), PPU_VIQ_HELD_OPS + 2, "{what}");
    let first = cycles[0];
    for (i, &c) in cycles.iter().enumerate().take(PPU_VIQ_HELD_OPS + 1) {
        assert!(
            c - first < PPU_FDIV_CYCLES,
            "{what}: divide {i} issues {} cycles after the first",
            c - first
        );
    }
    let past = cycles[PPU_VIQ_HELD_OPS + 1] - first;
    assert!(
        past >= PPU_FDIV_CYCLES,
        "{what}: the divide past the queue's room issues {past} cycles after the first"
    );
}

/// Sixteen independent `addi`, four registers in turn.
fn addis() -> Vec<I> {
    (0..16)
        .map(|i| I::Addi {
            rt: 17 + i % 4,
            ra: 17 + i % 4,
            imm: 1,
        })
        .collect()
}

/// A sibling whose stalls add `cycles` to each instruction, `divide` of
/// them and `fraction` / 256 more behind its divides.
fn sibling(cycles: u32, divide: u32, fraction: u32) -> PpeIssuePressure {
    PpeIssuePressure {
        cycles,
        vsu_cycles: cycles,
        multiply_cycles: cycles,
        divide_cycles: divide,
        divide_fraction: fraction,
        ..PpeIssuePressure::NONE
    }
}

/// The cycles from the first `addi` of [`addis`] to the last, beside
/// `pressure`.
fn addi_span(pressure: PpeIssuePressure) -> u64 {
    let mut timing = PpeTiming::new();
    timing.set_siblings(1, pressure);
    let cycles = run(&mut timing, &addis(), 4);
    cycles[15] - cycles[0]
}

#[test]
fn the_vsu_issue_queue_holds_six_dependent_divides_behind_the_one_that_runs() {
    let chain = vec![fdiv(); PPU_VIQ_HELD_OPS + 2];
    let mut timing = PpeTiming::new();
    assert_queue_holds(&run(&mut timing, &chain, 1), "dependent divides");
}

#[test]
fn the_vsu_issue_queue_holds_six_independent_divides_that_wait_for_the_fpu() {
    let chain: Vec<I> = (1..=PPU_VIQ_HELD_OPS as u8 + 2).map(fdiv_to).collect();
    let mut timing = PpeTiming::new();
    assert_queue_holds(&run(&mut timing, &chain, 1), "independent divides");
}

#[test]
fn a_fixed_point_op_beside_a_running_divide_waits_for_its_operand_at_issue() {
    // [CBE-Handbook p:756 s:A.6.2] the VSU issue queue holds VSU
    // instructions only.
    let mut timing = PpeTiming::new();
    let cycles = run(&mut timing, &[fdiv(), addi_r3(), addi_r3(), addi_r3()], 1);
    assert_eq!(cycles.len(), 4);
    for i in 2..4 {
        assert_eq!(
            cycles[i] - cycles[i - 1],
            PPU_FX_LATENCY_CYCLES,
            "addi {i} beside a running divide"
        );
    }
}

#[test]
fn with_no_fpu_divide_running_a_dependent_fp_op_waits_for_its_operand_at_issue() {
    // [CBE-Handbook p:320 s:10.7.2.9] the queue holds an op behind the
    // FPU's divide.
    for (what, lead) in [
        ("no divide", vec![]),
        ("a fixed-point divide", vec![divd()]),
    ] {
        let mut insns = lead.clone();
        insns.extend([fadd(); 3]);
        let mut timing = PpeTiming::new();
        let cycles = run(&mut timing, &insns, 1);
        let fadds = &cycles[lead.len()..];
        assert_eq!(fadds.len(), 3, "{what}");
        for i in 1..3 {
            assert_eq!(
                fadds[i] - fadds[i - 1],
                PPU_FP_LATENCY_CYCLES,
                "fadd {i} beside {what}"
            );
        }
    }
}

#[test]
fn a_stall_behind_a_sibling_s_divides_adds_past_the_dispatch_hold() {
    let none = addi_span(PpeIssuePressure::NONE);
    let other = addi_span(sibling(2, 0, 0));
    let divide = addi_span(sibling(2, 2, 0));
    assert!(
        divide > other && other > none,
        "beside no stall {none} cycles, a stall of other kinds {other}, a divide stall {divide}"
    );
}

#[test]
fn a_sibling_s_divide_fraction_carries_to_whole_cycles() {
    // The span holds the waits of the last 15 of 16 `addi`, and the carry
    // is zero at the first. At half a cycle each, every second `addi` waits
    // one cycle more: 8 in all. At three quarters, three of every four do:
    // 12 in all.
    let none = addi_span(PpeIssuePressure::NONE);
    let half = addi_span(sibling(0, 0, 128));
    let three_quarters = addi_span(sibling(0, 0, 192));
    assert_eq!(half - none, 8, "beside half a cycle");
    assert_eq!(
        three_quarters - none,
        12,
        "beside three quarters of a cycle"
    );
}

#[test]
fn a_divide_fraction_adds_no_cycle_past_the_shared_stall_s_cap() {
    let cap = PPU_SMT_SHARED_STALL_CYCLES as u32;
    assert_eq!(
        addi_span(sibling(cap, cap, 128)),
        addi_span(sibling(cap, cap, 0)),
        "beside a stall at the cap and half a cycle"
    );
}

#[test]
fn a_divide_fraction_carries_to_no_sibling_after_the_last_one_leaves() {
    // One `addi` beside half a cycle leaves half a cycle to carry.
    let fresh = addi_span(sibling(0, 0, 128));
    let mut timing = PpeTiming::new();
    timing.set_siblings(1, sibling(0, 0, 128));
    run(&mut timing, &addis()[..1], 1);
    timing.set_siblings(0, PpeIssuePressure::NONE);
    timing.set_siblings(1, sibling(0, 0, 128));
    let cycles = run(&mut timing, &addis(), 4);
    let rejoined = cycles[15] - cycles[0];
    assert_eq!(
        rejoined, fresh,
        "beside a new sibling {rejoined} cycles, beside the first one {fresh}"
    );
}
