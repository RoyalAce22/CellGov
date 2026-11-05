//! A fused `extsw; blr` stops where the two unfused instructions stop and
//! costs what they cost. One case is a `blr` back to its own slot.

use crate::*;
use cellgov_event::UnitId;
use cellgov_exec::{ExecutionContext, ExecutionUnit, YieldReason};
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory, PageSize, Region};
use cellgov_time::{Budget, InstructionCost};

use crate::instruction::{FusedPair, PpuInstruction};
use crate::state::PpuState;

/// The end of the one region, which starts at 0.
const FLAT_END: u64 = 0x1000;

const EXTSW_R3_R4: u32 = (31 << 26) | (4 << 21) | (3 << 16) | (986 << 1);
const BLR: u32 = 0x4E80_0020;
/// `addi r5, r5, 1`.
const ADDI_R5: u32 = (14 << 26) | (5 << 21) | (5 << 16) | 1;
const B_OVER_ONE: u32 = (18 << 26) | 8;

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

/// Runs `code` at 0 for six batches of `budget`, fused or by plain decode.
/// LR starts at `lr` and r4 starts at 0x8000_0000. Returns every batch's
/// stop and the state after the last batch.
fn run(code: &[u32], lr: u64, fused: bool, budget: u64) -> (Vec<Stop>, PpuState) {
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
    unit.state_mut().set_lr(lr);
    unit.state_mut().set_gpr(4, 0x8000_0000);
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
            Some(PpuInstruction::Fused(FusedPair::ExtswBlr { .. }))
        ),
        "precondition: the extsw and the blr fuse at {at:#x}"
    );
}

/// [PPC-Book1 p:25 s:2.4] the blr goes to LR, here the addi after the pair.
/// [PPC-Book1 p:69 s:3.3.11] extsw copies bit 32 of RS into bits 0:31 of RA.
#[test]
fn a_return_past_the_pair_runs_as_the_two_instructions() {
    let code = [EXTSW_R3_R4, BLR, ADDI_R5, ADDI_R5];
    assert_fuses(&code, 0);
    for budget in [1, 2, 3, 256] {
        assert_eq!(
            run(&code, 8, true, budget).0,
            run(&code, 8, false, budget).0,
            "budget {budget}"
        );
    }
    let (_, state) = run(&code, 8, true, 256);
    assert_eq!(state.gpr[3], 0xFFFF_FFFF_8000_0000, "extsw sign-extends");
    assert_eq!(state.gpr[5], 2);
}

/// LR names the blr's own slot, so the fused pair's next address is the
/// address after its first half. The batch and the issue model must read
/// that as the branch taken.
#[test]
fn a_blr_to_its_own_slot_costs_what_it_costs_unfused() {
    let code = [EXTSW_R3_R4, BLR];
    assert_fuses(&code, 0);
    for budget in [1, 2, 3, 7] {
        let (fused, state) = run(&code, 4, true, budget);
        assert_eq!(fused, run(&code, 4, false, budget).0, "budget {budget}");
        assert_eq!(state.pc, 4, "the blr loops on itself");
    }
}

#[test]
fn a_branch_into_the_blr_runs_it_alone() {
    let code = [B_OVER_ONE, EXTSW_R3_R4, BLR, ADDI_R5];
    assert_fuses(&code, 4);
    let (fused, state) = run(&code, 12, true, 2);
    assert_eq!(fused, run(&code, 12, false, 2).0);
    assert_eq!(state.gpr[3], 0, "the extsw did not run");
}
