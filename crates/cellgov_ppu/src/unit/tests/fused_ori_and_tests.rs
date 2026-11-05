//! A fused `ori; and` leaves the unit, and its timing, where the two
//! instructions leave them unfused.

use crate::*;
use cellgov_event::UnitId;
use cellgov_exec::{ExecutionContext, ExecutionUnit, YieldReason};
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_time::Budget;

use crate::instruction::encode::encode;
use crate::instruction::{FusedPair, PpuInstruction};

/// `addi r3, r3, 1`.
const ADDI_R3: u32 = (14 << 26) | (3 << 21) | (3 << 16) | 1;
/// `b` over one instruction.
const B_OVER_ONE: u32 = (18 << 26) | 8;

fn word(insn: PpuInstruction) -> u32 {
    encode(&insn).expect("encodes")
}

fn ori(ra: u8, rs: u8, imm: u16) -> u32 {
    word(PpuInstruction::Ori { ra, rs, imm })
}

fn and(ra: u8, rs: u8, rb: u8, rc: bool) -> u32 {
    word(PpuInstruction::And { ra, rs, rb, rc })
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

/// Runs `code` at 0, fused or by plain decode, in batches of `budget`,
/// until the PC passes the end of `code` or eight batches ran. Returns
/// the last batch's stop, with the cost summed over every batch.
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
    unit.state_mut().set_gpr(3, 0x8123_4567_89AB_CDEF);
    unit.state_mut().set_gpr(4, 0x0FED_CBA9_8765_4321);
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
            Some(PpuInstruction::Fused(FusedPair::OriAnd { .. }))
        ),
        "precondition: the pair fuses at {at:#x}"
    );
}

/// The cases cover:
/// - an ori kept raw, and one quickened to nop
/// - an and that records, and one that does not
/// - an and that reads the register the ori writes, and one that does not
#[test]
fn the_pair_runs_and_times_as_the_two_instructions() {
    for first in [ori(5, 4, 0x8001), ori(0, 0, 0), ori(4, 4, 0)] {
        for second in [and(6, 5, 3, false), and(6, 4, 3, true), and(0, 0, 3, true)] {
            let code = [ADDI_R3, first, second, ADDI_R3];
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
fn a_branch_into_the_and_runs_it_alone() {
    let code = [B_OVER_ONE, ori(5, 4, 1), and(6, 5, 3, false), ADDI_R3];
    assert_fuses(&code, 4);
    let fused = run(&code, true, 64);
    assert_eq!(fused, run(&code, false, 64));
    assert_eq!(fused.retired, 3, "b, and and addi; the ori did not run");
}
