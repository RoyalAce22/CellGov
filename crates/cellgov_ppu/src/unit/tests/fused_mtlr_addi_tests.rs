//! A fused `mtlr; addi` leaves the unit, and its timing, where the two
//! instructions leave them unfused.

use crate::*;
use cellgov_event::UnitId;
use cellgov_exec::{ExecutionContext, ExecutionUnit, YieldReason};
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_time::Budget;

use crate::instruction::{FusedPair, PpuInstruction};

/// `addi r3, r3, 1`.
const ADDI_R3: u32 = (14 << 26) | (3 << 21) | (3 << 16) | 1;
/// `addi r0, r3, 1`, which writes the register `mtlr r0` reads.
const ADDI_R0: u32 = (14 << 26) | (3 << 16) | 1;
/// `b` over one instruction.
const B_OVER_ONE: u32 = (18 << 26) | 8;
/// `mtlr r0`.
const MTLR_R0: u32 = (31 << 26) | (8 << 16) | (467 << 1);
/// `addi r1, r1, 112`.
const ADDI_R1: u32 = (14 << 26) | (1 << 21) | (1 << 16) | 112;
/// `li r4, -1`.
const LI_R4: u32 = (14 << 26) | (4 << 21) | 0xFFFF;

/// What a run left.
#[derive(Debug, PartialEq, Eq)]
struct Stop {
    reason: YieldReason,
    pc: Option<u64>,
    retired: u64,
    cost: u64,
    state: cellgov_exec::PpuFingerprint,
}

/// Runs `code` at 0, fused or by plain decode, in batches of `budget`.
///
/// The run stops at the first of:
/// - the PC passes the end of `code`
/// - eight batches ran
///
/// The result is the last batch's stop, with the cost of every batch.
fn run(code: &[u32], fused: bool, budget: u64) -> Stop {
    let bytes = code
        .iter()
        .flat_map(|w| w.to_be_bytes())
        .collect::<Vec<_>>();
    let mut mem = GuestMemory::new(0x100);
    mem.apply_commit(
        ByteRange::new(GuestAddr::new(0), bytes.len() as u64).unwrap(),
        &bytes,
    )
    .unwrap();
    let mut unit = PpuExecutionUnit::new(UnitId::new(0));
    if fused {
        unit.set_instruction_shadow(shadow::PredecodedShadow::build(0, &bytes));
    }
    unit.state_mut().set_gpr(0, 0x0001_0204);
    unit.state_mut().set_gpr(1, 0x7FF0);
    let mut cost = 0;
    let mut last = None;
    for _ in 0..8 {
        let r = unit.run_until_yield(
            Budget::new(budget),
            &ExecutionContext::new(&mem),
            &mut Vec::new(),
        );
        cost += r.consumed_cost.raw();
        last = Some((r.yield_reason, r.local_diagnostics.pc));
        if unit.state().pc >= 4 * code.len() as u64 {
            break;
        }
    }
    let (reason, pc) = last.expect("one batch ran");
    Stop {
        reason,
        pc,
        retired: unit.retired_instructions(),
        cost,
        state: unit.state().fingerprint(),
    }
}

fn assert_fuses(code: &[u32], at: u64) {
    let bytes = code
        .iter()
        .flat_map(|w| w.to_be_bytes())
        .collect::<Vec<_>>();
    assert!(
        matches!(
            shadow::PredecodedShadow::build(0, &bytes).get(at),
            Some(PpuInstruction::Fused(FusedPair::MtlrAddi { .. }))
        ),
        "precondition: the pair fuses at {at:#x}"
    );
}

/// The addi in its raw form and in its quickened li form, after an
/// instruction that writes the mtlr source and after one that does not.
#[test]
fn the_pair_runs_and_times_as_the_two_instructions() {
    for first in [ADDI_R3, ADDI_R0] {
        for second in [ADDI_R1, LI_R4] {
            let code = [first, MTLR_R0, second, ADDI_R3];
            assert_fuses(&code, 4);
            for budget in [1, 2, 3, 64] {
                assert_eq!(
                    run(&code, true, budget),
                    run(&code, false, budget),
                    "{first:#010x} then {second:#010x} budget {budget}"
                );
            }
        }
    }
}

#[test]
fn a_branch_into_the_addi_runs_it_alone() {
    let code = [B_OVER_ONE, MTLR_R0, ADDI_R1, ADDI_R3];
    assert_fuses(&code, 4);
    let fused = run(&code, true, 64);
    assert_eq!(fused, run(&code, false, 64));
    assert_eq!(fused.retired, 3, "b, addi and addi; the mtlr did not run");
}
