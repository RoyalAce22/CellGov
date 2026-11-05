//! A fused `rldicl; rldicl` leaves the unit, and its timing, where the two
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

/// The word for `rldicl rA, rS, sh, mb` with Rc `rc`.
fn rldicl(ra: u8, rs: u8, sh: u8, mb: u8, rc: bool) -> u32 {
    encode(&PpuInstruction::Rldicl { ra, rs, sh, mb, rc }).expect("rldicl encodes")
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
/// the last batch's stop, with the cost of every batch.
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
            Some(PpuInstruction::Fused(FusedPair::RldiclRldicl { .. }))
        ),
        "precondition: the pair fuses at {at:#x}"
    );
}

/// The cases give each half every one of these forms:
/// - raw
/// - quickened to `srdi` or `clrldi`
/// - recording
///
/// The second half reads the register that the first half writes.
#[test]
fn the_pair_runs_and_times_as_the_two_instructions() {
    let firsts = [
        rldicl(5, 4, 8, 4, false),
        rldicl(5, 4, 60, 4, false),
        rldicl(5, 3, 0, 0, true),
    ];
    let seconds = [
        rldicl(6, 5, 0, 32, false),
        rldicl(6, 5, 56, 8, false),
        rldicl(6, 5, 1, 1, true),
    ];
    for first in firsts {
        for second in seconds {
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
fn a_branch_into_the_second_rldicl_runs_it_alone() {
    let code = [
        B_OVER_ONE,
        rldicl(5, 4, 8, 4, false),
        rldicl(6, 5, 0, 32, false),
        ADDI_R3,
    ];
    assert_fuses(&code, 4);
    let fused = run(&code, true, 64);
    assert_eq!(fused, run(&code, false, 64));
    assert_eq!(
        fused.retired, 3,
        "b, rldicl and addi; the first rldicl did not run"
    );
}
