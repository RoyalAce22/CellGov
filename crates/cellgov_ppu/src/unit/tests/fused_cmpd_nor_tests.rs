//! A fused `cmpd; nor` runs as the two instructions do unfused, and a
//! branch into the `nor` runs it alone.

use crate::*;
use cellgov_event::UnitId;
use cellgov_exec::{ExecutionContext, ExecutionUnit, YieldReason};
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory, PageSize, Region};
use cellgov_time::{Budget, InstructionCost};

use crate::instruction::{FusedPair, PpuInstruction};
use crate::state::PpuState;

/// `cmpd cr7, r3, r4`.
const CMPD_CR7: u32 = (31 << 26) | (7 << 23) | (1 << 21) | (3 << 16) | (4 << 11);
/// `nor r5, r6, r6`.
const NOR_R5: u32 = (31 << 26) | (6 << 21) | (5 << 16) | (6 << 11) | (124 << 1);
/// `addi r9, r9, 1`.
const ADDI_R9: u32 = (14 << 26) | (9 << 21) | (9 << 16) | 1;
const B_OVER_ONE: u32 = (18 << 26) | 8;

/// What a run left.
#[derive(Debug, PartialEq, Eq)]
struct Stop {
    reason: YieldReason,
    cost: InstructionCost,
    retired: u64,
    state: cellgov_exec::PpuFingerprint,
    state_pc: u64,
}

/// Runs `code` at 0 in one batch of `budget`, fused or by plain decode,
/// with r3 = 0x1_0000_0000, r4 = 1 and r6 = 0xF0.
fn run(code: &[u32], fused: bool, budget: u64) -> (Stop, PpuState) {
    let bytes = code
        .iter()
        .flat_map(|w| w.to_be_bytes())
        .collect::<Vec<_>>();
    let mut mem =
        GuestMemory::from_regions(vec![Region::new(0, 0x1000, "flat", PageSize::Page4K)]).unwrap();
    mem.apply_commit(
        ByteRange::new(GuestAddr::new(0), bytes.len() as u64).unwrap(),
        &bytes,
    )
    .unwrap();
    let mut unit = PpuExecutionUnit::new(UnitId::new(0));
    if fused {
        unit.set_instruction_shadow(shadow::PredecodedShadow::build(0, &bytes));
    }
    unit.state_mut().set_gpr(3, 0x1_0000_0000);
    unit.state_mut().set_gpr(4, 1);
    unit.state_mut().set_gpr(6, 0xF0);
    let r = unit.run_until_yield(
        Budget::new(budget),
        &ExecutionContext::new(&mem),
        &mut Vec::new(),
    );
    let stop = Stop {
        reason: r.yield_reason,
        cost: r.consumed_cost,
        retired: unit.retired_instructions(),
        state: unit.state().fingerprint(),
        state_pc: unit.state().pc,
    };
    (stop, unit.state().clone())
}

fn assert_fuses(code: &[u32], at: u64) {
    let bytes = code
        .iter()
        .flat_map(|w| w.to_be_bytes())
        .collect::<Vec<_>>();
    assert!(
        matches!(
            shadow::PredecodedShadow::build(0, &bytes).get(at),
            Some(PpuInstruction::Fused(FusedPair::CmpdNor { .. }))
        ),
        "precondition: the cmpd and the nor fuse at {at:#x}"
    );
}

#[test]
fn a_fused_cmpd_nor_runs_as_the_two_instructions() {
    let code = [CMPD_CR7, NOR_R5, ADDI_R9];
    assert_fuses(&code, 0);
    for budget in [1, 2, 3] {
        assert_eq!(
            run(&code, true, budget).0,
            run(&code, false, budget).0,
            "budget {budget}"
        );
    }
    let (_, state) = run(&code, true, 3);
    assert_eq!(state.cr_field(7), 0b0100, "cr7 is GT");
    assert_eq!(state.gpr[5], !0xF0);
}

#[test]
fn a_branch_into_the_nor_runs_it_alone() {
    let code = [B_OVER_ONE, CMPD_CR7, NOR_R5, ADDI_R9];
    assert_fuses(&code, 4);
    let (fused, state) = run(&code, true, 3);
    assert_eq!(fused, run(&code, false, 3).0);
    assert_eq!(state.cr_field(7), 0, "the cmpd did not run");
    assert_eq!(state.gpr[5], !0xF0);
}
