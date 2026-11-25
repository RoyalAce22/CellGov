use super::{classify, PpeTiming};
use crate::instruction::ops::Fp63Op;
use crate::instruction::PpuInstruction as I;
use crate::state::PpuState;
use cellgov_ps3_abi::hw::ppu_issue::{PPU_FDIV_CYCLES, PPU_VIQ_HELD_OPS};

/// Retire `insns` once as straight-line code from 0x1000, and return the
/// issue cycle of each op.
fn run(insns: &[I]) -> Vec<u64> {
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

/// `addi r21,r21,1`.
fn addi() -> I {
    I::Addi {
        rt: 21,
        ra: 21,
        imm: 1,
    }
}

/// The issue cycle of each `fdiv`, counted from the first, in a chain of
/// `n` dependent `fdiv` with `k` `addi` after each.
fn divide_issues(n: usize, k: usize) -> Vec<u64> {
    let mut insns = Vec::new();
    for _ in 0..n {
        insns.push(fdiv());
        insns.extend(std::iter::repeat_n(addi(), k));
    }
    let cycles = run(&insns);
    cycles
        .iter()
        .step_by(k + 1)
        .map(|c| c - cycles[0])
        .collect()
}

#[test]
fn an_addi_after_each_divide_takes_a_stage_of_the_queue_behind_it() {
    // One divide runs. Each later divide takes a stage, and so does the
    // `addi` after it: three divides fit in the six stages. The fifth
    // divide waits for the oldest stage to leave, one divide later.
    let held = PPU_VIQ_HELD_OPS / 2;
    let issues = divide_issues(held + 2, 1);
    for (i, &at) in issues.iter().enumerate().take(held + 1) {
        assert!(
            at < PPU_FDIV_CYCLES,
            "divide {i} issues {at} cycles after the first"
        );
    }
    let past = issues[held + 1];
    assert!(
        past >= PPU_FDIV_CYCLES,
        "the divide past the stages issues {past} cycles after the first"
    );
}

#[test]
fn back_to_back_divides_each_take_one_stage() {
    let issues = divide_issues(PPU_VIQ_HELD_OPS + 2, 0);
    assert!(issues[PPU_VIQ_HELD_OPS] < PPU_FDIV_CYCLES, "{issues:?}");
    assert!(
        issues[PPU_VIQ_HELD_OPS + 1] >= PPU_FDIV_CYCLES,
        "{issues:?}"
    );
}

#[test]
fn with_no_divide_held_an_addi_takes_no_stage() {
    // The queue holds nothing yet, so six divides still fit behind the
    // one that runs.
    let mut insns = vec![addi(); 8];
    insns.extend(std::iter::repeat_n(fdiv(), PPU_VIQ_HELD_OPS + 2));
    let cycles = run(&insns);
    let first = cycles[8];
    assert!(first <= cycles[7] + 1, "{cycles:?}");
    assert!(
        cycles[8 + PPU_VIQ_HELD_OPS] - first < PPU_FDIV_CYCLES,
        "{cycles:?}"
    );
}
