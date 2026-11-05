//! A fused `ldu; ldu` stops where the two loads stop unfused.
//!
//! [PPC-Book3 p:61 s:5.2] when an interrupt occurs, every preceding
//! instruction is complete, so a second load that faults leaves the
//! first load's result and its base update in place.

use crate::*;
use cellgov_event::UnitId;
use cellgov_exec::{ExecutionContext, ExecutionUnit, YieldReason};
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory, PageSize, Region};
use cellgov_time::Budget;

use crate::instruction::{FusedPair, PpuInstruction};
use crate::state::PpuState;

/// The end of the one region, which starts at 0.
const FLAT_END: u64 = 0x1000;
/// The doublewords at `FLAT_END - 16` and `FLAT_END - 8`.
const SECOND_LAST: u64 = 0x1111_2222_3333_4444;
const LAST: u64 = 0x5555_6666_7777_8888;

const B_OVER_ONE: u32 = (18 << 26) | 8;
/// `addi r3, r3, 1`.
const ADDI_R3: u32 = (14 << 26) | (3 << 21) | (3 << 16) | 1;

/// `ldu rT, d(rA)`.
fn ldu(rt: u32, ra: u32, d: i16) -> u32 {
    (58 << 26) | (rt << 21) | (ra << 16) | (u32::from(d as u16) & 0xFFFC) | 1
}

/// What a run left.
#[derive(Debug, PartialEq, Eq)]
struct Stop {
    reason: YieldReason,
    pc: Option<u64>,
    faulting_ea: Option<u64>,
    retired: u64,
    state: cellgov_exec::PpuFingerprint,
    state_pc: u64,
}

/// Runs `code` at 0 with r5 = `r5` and r6 = `r6`, fused or by plain
/// decode, in batches of `budget`.
///
/// The run stops at the first of:
/// - a batch that faults
/// - a PC past the end of `code`
/// - the eighth batch
///
/// Returns the last batch's stop and the state it left.
fn run(code: &[u32], (r5, r6): (u64, u64), fused: bool, budget: u64) -> (Stop, PpuState) {
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
    let tail = [SECOND_LAST.to_be_bytes(), LAST.to_be_bytes()].concat();
    mem.apply_commit(
        ByteRange::new(GuestAddr::new(FLAT_END - 16), 16).unwrap(),
        &tail,
    )
    .unwrap();
    let mut unit = PpuExecutionUnit::new(UnitId::new(0));
    if fused {
        unit.set_instruction_shadow(shadow::PredecodedShadow::build(0, &bytes));
    }
    unit.state_mut().set_gpr(5, r5);
    unit.state_mut().set_gpr(6, r6);
    unit.state_mut().set_gpr(7, 0x77);
    unit.state_mut().set_gpr(8, 0x88);
    let mut stop = None;
    for _ in 0..8 {
        let r = unit.run_until_yield(
            Budget::new(budget),
            &ExecutionContext::new(&mem),
            &mut Vec::new(),
        );
        let fault = r.yield_reason == YieldReason::Fault;
        stop = Some(Stop {
            reason: r.yield_reason,
            pc: r.local_diagnostics.pc,
            faulting_ea: r.local_diagnostics.faulting_ea,
            retired: unit.retired_instructions(),
            state: unit.state().fingerprint(),
            state_pc: unit.state().pc,
        });
        if fault || unit.state().pc >= 4 * code.len() as u64 {
            break;
        }
    }
    (stop.expect("one batch ran"), unit.state().clone())
}

fn assert_fuses(code: &[u32], at: u64) {
    let bytes = code
        .iter()
        .flat_map(|w| w.to_be_bytes())
        .collect::<Vec<_>>();
    assert!(
        matches!(
            shadow::PredecodedShadow::build(0, &bytes).get(at),
            Some(PpuInstruction::Fused(FusedPair::LduLdu { .. }))
        ),
        "precondition: the two ldu fuse at {at:#x}"
    );
}

#[test]
fn two_loads_on_one_base_walk_it_twice() {
    let code = [ldu(7, 5, 8), ldu(8, 5, 8)];
    assert_fuses(&code, 0);
    let start = (FLAT_END - 24, 0);
    let (fused, state) = run(&code, start, true, 2);
    assert_eq!(fused, run(&code, start, false, 2).0);
    assert_eq!(state.gpr[7], SECOND_LAST);
    assert_eq!(state.gpr[8], LAST);
    assert_eq!(state.gpr[5], FLAT_END - 8);
}

#[test]
fn two_loads_on_two_bases_update_each() {
    let code = [ldu(7, 5, 8), ldu(8, 6, 8)];
    assert_fuses(&code, 0);
    let start = (FLAT_END - 24, FLAT_END - 16);
    let (fused, state) = run(&code, start, true, 2);
    assert_eq!(fused, run(&code, start, false, 2).0);
    assert_eq!(state.gpr[7], SECOND_LAST);
    assert_eq!(state.gpr[8], LAST);
    assert_eq!(state.gpr[5], FLAT_END - 16);
    assert_eq!(state.gpr[6], FLAT_END - 8);
}

#[test]
fn a_second_load_into_the_first_base_overwrites_its_update() {
    let code = [ldu(7, 5, 8), ldu(5, 6, 8)];
    assert_fuses(&code, 0);
    let start = (FLAT_END - 24, FLAT_END - 16);
    let (fused, state) = run(&code, start, true, 2);
    assert_eq!(fused, run(&code, start, false, 2).0);
    assert_eq!(state.gpr[7], SECOND_LAST);
    assert_eq!(state.gpr[5], LAST);
    assert_eq!(state.gpr[6], FLAT_END - 8);
}

#[test]
fn two_loads_into_one_target_leave_the_second_value() {
    let code = [ldu(7, 5, 8), ldu(7, 6, 8)];
    assert_fuses(&code, 0);
    let start = (FLAT_END - 24, FLAT_END - 16);
    let (fused, state) = run(&code, start, true, 2);
    assert_eq!(fused, run(&code, start, false, 2).0);
    assert_eq!(state.gpr[7], LAST);
    assert_eq!(state.gpr[5], FLAT_END - 16);
    assert_eq!(state.gpr[6], FLAT_END - 8);
}

/// The fused pair writes nothing before it refuses, so the split rerun
/// starts from the original r5.
#[test]
fn a_second_load_off_the_region_faults_after_the_first_retires() {
    let code = [ADDI_R3, ldu(7, 5, 8), ldu(8, 5, 8), ADDI_R3];
    assert_fuses(&code, 4);
    let start = (FLAT_END - 16, 0);
    for budget in [1, 2, 3, 256] {
        assert_eq!(
            run(&code, start, true, budget).0,
            run(&code, start, false, budget).0,
            "budget {budget}"
        );
    }
    let (stop, state) = run(&code, start, true, 256);
    assert_eq!(stop.reason, YieldReason::Fault);
    assert_eq!(
        stop.pc,
        Some(8),
        "the second load faults at its own address"
    );
    assert_eq!(stop.faulting_ea, Some(FLAT_END));
    assert_eq!(stop.retired, 2, "addi and the first load retired");
    assert_eq!(state.gpr[7], LAST, "the first load's value");
    assert_eq!(state.gpr[5], FLAT_END - 8, "r5 moved once");
    assert_eq!(state.gpr[8], 0x88, "the second load wrote nothing");
}

#[test]
fn a_first_load_off_the_region_faults_with_nothing_written() {
    let code = [ldu(7, 5, 8), ldu(8, 6, 0)];
    assert_fuses(&code, 0);
    let start = (FLAT_END - 4, FLAT_END - 8);
    assert_eq!(run(&code, start, true, 2).0, run(&code, start, false, 2).0);
    let (stop, state) = run(&code, start, true, 2);
    assert_eq!(stop.reason, YieldReason::Fault);
    assert_eq!(stop.pc, Some(0));
    assert_eq!(stop.faulting_ea, Some(FLAT_END + 4), "EA1 = r5 + 8");
    assert_eq!(stop.retired, 0, "neither load retired");
    assert_eq!(state.gpr[5], FLAT_END - 4);
    assert_eq!(state.gpr[6], FLAT_END - 8);
}

#[test]
fn a_branch_into_the_second_load_runs_it_alone() {
    let code = [B_OVER_ONE, ldu(7, 5, 8), ldu(8, 6, 8)];
    assert_fuses(&code, 4);
    let start = (FLAT_END - 24, FLAT_END - 16);
    let (fused, state) = run(&code, start, true, 2);
    assert_eq!(fused, run(&code, start, false, 2).0);
    assert_eq!(state.gpr[7], 0x77, "the first load did not run");
    assert_eq!(state.gpr[5], FLAT_END - 24);
    assert_eq!(state.gpr[8], LAST);
}
