//! A fused `or; rlwinm` stops where the two unfused instructions stop.

use crate::*;
use cellgov_event::UnitId;
use cellgov_exec::{ExecutionContext, ExecutionUnit, YieldReason};
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory, PageSize, Region};
use cellgov_time::{Budget, InstructionCost};

use crate::instruction::{FusedPair, PpuInstruction};
use crate::state::PpuState;

/// The end of the one region, which starts at 0.
const FLAT_END: u64 = 0x1000;

const B_OVER_ONE: u32 = (18 << 26) | 8;
/// `or r3, r4, r5`.
const OR_R3: u32 = (31 << 26) | (4 << 21) | (3 << 16) | (5 << 11) | (444 << 1);
/// `rlwinm. r6, r3, 4, 0, 27`.
const RLWINM_DOT_R6: u32 = (21 << 26) | (3 << 21) | (6 << 16) | (4 << 11) | (27 << 1) | 1;

/// What one batch left.
#[derive(Debug, PartialEq, Eq)]
struct Stop {
    reason: YieldReason,
    pc: Option<u64>,
    cost: InstructionCost,
    retired: u64,
    state: cellgov_exec::PpuFingerprint,
    state_pc: u64,
}

/// Runs `code` at 0 for four batches of `budget`, fused or by plain
/// decode, with r3 = 0x33, r4 = 0x40 and r5 = 0x05. Returns every batch's
/// stop and the state after the last batch.
fn run(code: &[u32], fused: bool, budget: u64) -> (Vec<Stop>, PpuState) {
    let bytes = code
        .iter()
        .flat_map(|w| w.to_be_bytes())
        .collect::<Vec<_>>();
    let mut mem = GuestMemory::from_regions(vec![Region::new(
        0,
        FLAT_END as usize,
        "flat",
        PageSize::Page4K,
    )])
    .unwrap();
    mem.apply_commit(
        ByteRange::new(GuestAddr::new(0), bytes.len() as u64).unwrap(),
        &bytes,
    )
    .unwrap();
    let mut unit = PpuExecutionUnit::new(UnitId::new(0));
    if fused {
        unit.set_instruction_shadow(shadow::PredecodedShadow::build(0, &bytes));
    }
    unit.state_mut().set_gpr(3, 0x33);
    unit.state_mut().set_gpr(4, 0x40);
    unit.state_mut().set_gpr(5, 0x05);
    let mut stops = Vec::new();
    for _ in 0..4 {
        let r = unit.run_until_yield(
            Budget::new(budget),
            &ExecutionContext::new(&mem),
            &mut Vec::new(),
        );
        stops.push(Stop {
            reason: r.yield_reason,
            pc: r.local_diagnostics.pc,
            cost: r.consumed_cost,
            retired: unit.retired_instructions(),
            state: unit.state().fingerprint(),
            state_pc: unit.state().pc,
        });
    }
    (stops, unit.state().clone())
}

fn assert_fuses(code: &[u32], at: u64) {
    let bytes = code
        .iter()
        .flat_map(|w| w.to_be_bytes())
        .collect::<Vec<_>>();
    assert!(
        matches!(
            shadow::PredecodedShadow::build(0, &bytes).get(at),
            Some(PpuInstruction::Fused(FusedPair::OrRlwinm { .. }))
        ),
        "precondition: the or and the rlwinm fuse at {at:#x}"
    );
}

/// [PPC-Book1 p:73 s:3.3.12] 0x40 | 0x05 = 0x45, rotated left 4 to 0x450.
#[test]
fn the_pair_runs_as_the_two_instructions() {
    let code = [OR_R3, RLWINM_DOT_R6];
    assert_fuses(&code, 0);
    for budget in [1, 2, 3, 256] {
        assert_eq!(
            run(&code, true, budget).0,
            run(&code, false, budget).0,
            "budget {budget}"
        );
    }
    let (_, state) = run(&code, true, 256);
    assert_eq!((state.gpr[3], state.gpr[6]), (0x45, 0x450));
    assert_eq!(state.cr_field(0), 0b0100);
}

#[test]
fn a_branch_into_the_rlwinm_runs_it_alone() {
    let code = [B_OVER_ONE, OR_R3, RLWINM_DOT_R6];
    assert_fuses(&code, 4);
    let (fused, state) = run(&code, true, 256);
    assert_eq!(fused, run(&code, false, 256).0);
    assert_eq!(state.gpr[3], 0x33, "the or did not run");
    assert_eq!(state.gpr[6], 0x330);
}
