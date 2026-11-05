//! A fused `addi; bc` stops where the two unfused instructions stop and
//! costs what they cost.

use crate::*;
use cellgov_event::UnitId;
use cellgov_exec::{ExecutionContext, ExecutionUnit, YieldReason};
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory, PageSize, Region};
use cellgov_time::{Budget, InstructionCost};

use crate::instruction::{FusedPair, PpuInstruction};
use crate::state::PpuState;

/// The end of the one region, which starts at 0.
const FLAT_END: u64 = 0x1000;

/// `addi r5, r5, 1`.
const ADDI_R5: u32 = (14 << 26) | (5 << 21) | (5 << 16) | 1;
/// `addi r6, r6, 1`.
const ADDI_R6: u32 = (14 << 26) | (6 << 21) | (6 << 16) | 1;
const B_OVER_ONE: u32 = (18 << 26) | 8;

/// `bdnz` (BO 10000) with displacement `offset` from its own address.
fn bdnz(offset: i16) -> u32 {
    (16 << 26) | (0b10000 << 21) | u32::from(offset as u16)
}

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

/// Runs `code` at 0 for six batches of `budget`, fused or by plain decode,
/// with CTR starting at 3. Returns every batch's stop and the state after
/// the last batch.
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
    unit.state_mut().set_ctr(3);
    let mut stops = Vec::new();
    for _ in 0..6 {
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
            Some(PpuInstruction::Fused(FusedPair::AddiBc { .. }))
        ),
        "precondition: the addi and the bc fuse at {at:#x}"
    );
}

/// [PPC-Book1 p:24 s:2.4] the bdnz goes back to the addi while CTR,
/// decremented, is not 0: the loop runs three times.
#[test]
fn a_counted_loop_runs_as_the_two_instructions() {
    let code = [ADDI_R5, bdnz(-4), ADDI_R6];
    assert_fuses(&code, 0);
    for budget in [1, 2, 3, 256] {
        assert_eq!(
            run(&code, true, budget).0,
            run(&code, false, budget).0,
            "budget {budget}"
        );
    }
    let (_, state) = run(&code, true, 256);
    assert_eq!((state.gpr[5], state.gpr[6], state.ctr()), (3, 1, 0));
}

/// The bdnz names its own slot, so the fused pair's next address is the
/// address after its first half. The batch and the issue model must read
/// that as the branch taken.
#[test]
fn a_bdnz_to_its_own_slot_costs_what_it_costs_unfused() {
    let code = [ADDI_R5, bdnz(0), ADDI_R6];
    assert_fuses(&code, 0);
    for budget in [1, 2, 3, 7] {
        let (fused, state) = run(&code, true, budget);
        assert_eq!(fused, run(&code, false, budget).0, "budget {budget}");
        assert_eq!(state.gpr[5], 1, "the addi runs once");
    }
}

#[test]
fn a_branch_into_the_bdnz_runs_it_alone() {
    let code = [B_OVER_ONE, ADDI_R5, bdnz(-8), ADDI_R6];
    assert_fuses(&code, 4);
    let (fused, state) = run(&code, true, 256);
    assert_eq!(fused, run(&code, false, 256).0);
    assert_eq!(state.gpr[5], 0, "the addi did not run");
    assert_eq!((state.gpr[6], state.ctr()), (1, 0));
}
