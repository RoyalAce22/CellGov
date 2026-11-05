//! Every internal path charges the guest ticks the plain path charges: a
//! quickened or fused form costs what its raw instructions cost.

use cellgov_fuzz::ppu_paths::{first_path_divergence, run_all_paths, PpuExecutionPath};
use cellgov_ppu::instruction::{FusedPair, PpuInstruction};
use cellgov_ppu::shadow::PredecodedShadow;
use cellgov_ppu::state::PpuState;

const DATA_BASE: u64 = 0x1000_0000;

fn cmpwi(ra: u32, imm: u16) -> u32 {
    (11 << 26) | (ra << 16) | u32::from(imm)
}

fn cmpw(ra: u32, rb: u32) -> u32 {
    (31 << 26) | (ra << 16) | (rb << 11)
}

/// `beq` (BO 12, BI 2: CR0 EQ, no hint) to `bd` bytes past itself.
fn beq(bd: u16) -> u32 {
    (16 << 26) | (12 << 21) | (2 << 16) | u32::from(bd & 0xfffc)
}

fn addi(rt: u32, ra: u32, imm: u16) -> u32 {
    (14 << 26) | (rt << 21) | (ra << 16) | u32::from(imm)
}

fn lwz(rt: u32, ra: u32, d: u16) -> u32 {
    (32 << 26) | (rt << 21) | (ra << 16) | u32::from(d)
}

fn stw(rs: u32, ra: u32, d: u16) -> u32 {
    (36 << 26) | (rs << 21) | (ra << 16) | u32::from(d)
}

const NOP: u32 = 24 << 26;

fn state(r3: u64) -> PpuState {
    let mut state = PpuState::new();
    state.set_gpr(3, r3);
    state.set_gpr(4, DATA_BASE);
    state
}

fn fused_at(words: &[u32], pc: u64) -> Option<PpuInstruction> {
    let code = words
        .iter()
        .flat_map(|word| word.to_be_bytes())
        .collect::<Vec<_>>();
    PredecodedShadow::build(0, &code).get(pc)
}

/// Runs `words` on every path and returns each path's cost, after it
/// checks that no path differs from another.
fn costs(words: &[u32], state: &PpuState) -> Vec<(PpuExecutionPath, Option<u64>)> {
    let runs = run_all_paths(words, state, &[0; 64]).expect("the case runs");
    assert!(
        first_path_divergence(&runs).is_none(),
        "{words:08x?}: {:#?}",
        first_path_divergence(&runs)
    );
    runs.iter()
        .map(|run| (run.path, run.consumed_cost))
        .collect()
}

/// A taken `beq .` spins on its own slot and uses the whole budget. The
/// fused path runs the pair once, then the `beq` alone.
#[test]
fn a_fused_compare_and_branch_to_its_own_slot_costs_what_the_plain_path_costs() {
    for first in [cmpwi(3, 0), cmpw(3, 5)] {
        let words = [first, beq(0), NOP, NOP, NOP, NOP, NOP, NOP];
        assert!(matches!(
            fused_at(&words, 0),
            Some(PpuInstruction::Fused(
                FusedPair::CmpwiBc { .. } | FusedPair::CmpwBc { .. }
            ))
        ));
        for r3 in [0, 1] {
            let costs = costs(&words, &state(r3));
            assert_eq!(costs.len(), 4);
            assert!(
                costs[0].1.is_some_and(|cost| cost > 0),
                "the plain path is priced: {costs:?}"
            );
            assert!(
                costs.iter().all(|(_, cost)| *cost == costs[0].1),
                "every path costs what the plain path costs: {costs:?}"
            );
        }
    }
}

#[test]
fn fused_and_quickened_code_costs_what_the_plain_path_costs() {
    let words = [
        addi(5, 0, 7),
        stw(5, 4, 0),
        lwz(6, 4, 0),
        cmpwi(6, 7),
        beq(8),
        addi(7, 7, 1),
        cmpw(6, 5),
        beq(0),
        NOP,
        NOP,
    ];
    let heads = (0..words.len() as u64)
        .filter_map(|i| match fused_at(&words, 4 * i) {
            Some(PpuInstruction::Fused(pair)) => Some(pair),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert!(
        heads.len() >= 3,
        "precondition: three pairs fuse: {heads:?}"
    );
    assert!(
        heads
            .iter()
            .any(|pair| matches!(pair, FusedPair::CmpwBc { .. })),
        "precondition: the cmpw and the beq fuse: {heads:?}"
    );
    let costs = costs(&words, &state(0));
    assert_eq!(costs.len(), 4);
    assert!(
        costs[0].1.is_some_and(|cost| cost > 0),
        "the plain path is priced: {costs:?}"
    );
    assert!(
        costs.iter().all(|(_, cost)| *cost == costs[0].1),
        "every path costs what the plain path costs: {costs:?}"
    );
}

#[test]
fn a_cost_difference_alone_is_a_divergence() {
    let words = [cmpwi(3, 0), beq(0), NOP, NOP];
    let mut runs = run_all_paths(&words, &state(0), &[0; 64]).expect("the case runs");
    assert_eq!(first_path_divergence(&runs), None);
    assert_eq!(runs[3].path, PpuExecutionPath::Fused);
    runs[3].consumed_cost = runs[3].consumed_cost.map(|cost| cost + 1);
    let divergence = first_path_divergence(&runs).expect("the costs differ");
    assert_eq!(
        (divergence.left, divergence.right),
        (PpuExecutionPath::Plain, PpuExecutionPath::Fused)
    );
    assert!(divergence.cost_differs);
    assert!(divergence.observation.is_empty());
    assert!(!divergence.stop_differs && !divergence.retired_differs);
    assert!(!divergence.data_reads_differ);
}
