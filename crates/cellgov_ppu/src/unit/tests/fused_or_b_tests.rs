//! A fused `or; b` leaves the unit, and its timing, where the two
//! instructions leave them unfused.

use crate::*;
use cellgov_event::UnitId;
use cellgov_exec::{ExecutionContext, ExecutionUnit, YieldReason};
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_time::Budget;

use crate::instruction::{FusedPair, PpuInstruction};

/// `addi r3, r3, 1`.
const ADDI_R3: u32 = (14 << 26) | (3 << 21) | (3 << 16) | 1;
/// `b` over one instruction.
const B_OVER_ONE: u32 = (18 << 26) | 8;

/// `or r6, r4, r5`.
const OR_R6: u32 = (31 << 26) | (4 << 21) | (6 << 16) | (5 << 11) | (444 << 1);

/// `b` to `offset` bytes from its own address.
fn b(offset: i32) -> u32 {
    (18 << 26) | (offset as u32 & 0x03FF_FFFC)
}

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
    unit.state_mut().set_gpr(4, 0x40);
    unit.state_mut().set_gpr(5, 0x05);
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
            Some(PpuInstruction::Fused(FusedPair::OrB { .. }))
        ),
        "precondition: the pair fuses at {at:#x}"
    );
}

/// The b offsets:
/// - 4 lands on the next instruction
/// - 8 skips one instruction
/// - -8 jumps back to the entry
#[test]
fn the_pair_runs_and_times_as_the_two_instructions() {
    for (offset, code_len) in [(4, 4), (8, 4), (-8, 4)] {
        let mut code = vec![ADDI_R3, OR_R6, b(offset), ADDI_R3];
        code.truncate(code_len);
        assert_fuses(&code, 4);
        for budget in [1, 2, 3, 64] {
            assert_eq!(
                run(&code, true, budget),
                run(&code, false, budget),
                "b {offset} budget {budget}"
            );
        }
    }
}

#[test]
fn a_branch_into_the_b_runs_it_alone() {
    let code = [B_OVER_ONE, OR_R6, b(8), ADDI_R3, ADDI_R3];
    assert_fuses(&code, 4);
    let fused = run(&code, true, 64);
    assert_eq!(fused, run(&code, false, 64));
    assert_eq!(fused.retired, 3, "b, b and one addi; the or did not run");
}
