//! A fused `lwz; lwz` stops where the two loads stop unfused.
//!
//! [PPC-Book3 p:61 s:5.2] when an interrupt occurs, every preceding
//! instruction is complete, so a second load that faults leaves the
//! first load's result in place.

use crate::*;
use cellgov_event::UnitId;
use cellgov_exec::{ExecutionContext, ExecutionUnit, YieldReason};
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory, PageSize, Region};
use cellgov_time::Budget;

use crate::instruction::{FusedPair, PpuInstruction};
use crate::state::PpuState;

/// The end of the one region, which starts at 0.
const FLAT_END: u64 = 0x1000;
/// The words at `FLAT_END - 8` and `FLAT_END - 4`.
const LAST_WORDS: [u32; 2] = [0x0102_0304, 0x0506_0708];

const B_OVER_ONE: u32 = (18 << 26) | 8;
/// `addi r3, r3, 1`.
const ADDI_R3: u32 = (14 << 26) | (3 << 21) | (3 << 16) | 1;

/// `lwz rT, d(rA)`.
fn lwz(rt: u32, ra: u32, d: i16) -> u32 {
    (32 << 26) | (rt << 21) | (ra << 16) | u32::from(d as u16)
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

/// Runs `code` at 0 with r5 = `r5`, fused or by plain decode, in batches
/// of `budget`, until a batch faults, the PC passes the end of `code`, or
/// eight batches ran. Returns the last batch's stop and the state it left.
fn run(code: &[u32], r5: u64, fused: bool, budget: u64) -> (Stop, PpuState) {
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
    let tail = LAST_WORDS
        .iter()
        .flat_map(|w| w.to_be_bytes())
        .collect::<Vec<_>>();
    mem.apply_commit(
        ByteRange::new(GuestAddr::new(FLAT_END - 8), 8).unwrap(),
        &tail,
    )
    .unwrap();
    let mut unit = PpuExecutionUnit::new(UnitId::new(0));
    if fused {
        unit.set_instruction_shadow(shadow::PredecodedShadow::build(0, &bytes));
    }
    unit.state_mut().set_gpr(5, r5);
    unit.state_mut().set_gpr(6, 0x66);
    unit.state_mut().set_gpr(7, 0x77);
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
            Some(PpuInstruction::Fused(FusedPair::LwzLwz { .. }))
        ),
        "precondition: the loads fuse at {at:#x}"
    );
}

#[test]
fn a_second_load_off_the_region_faults_after_the_first_retires() {
    let code = [ADDI_R3, lwz(6, 5, 4), lwz(7, 5, 8), ADDI_R3];
    assert_fuses(&code, 4);
    for budget in [1, 2, 3, 256] {
        assert_eq!(
            run(&code, FLAT_END - 8, true, budget).0,
            run(&code, FLAT_END - 8, false, budget).0,
            "budget {budget}"
        );
    }
    let (stop, state) = run(&code, FLAT_END - 8, true, 256);
    assert_eq!(stop.reason, YieldReason::Fault);
    assert_eq!(
        stop.pc,
        Some(8),
        "the second load faults at its own address"
    );
    assert_eq!(stop.faulting_ea, Some(FLAT_END));
    assert_eq!(stop.retired, 2, "addi and the first load retired");
    assert_eq!(state.gpr[6], u64::from(LAST_WORDS[1]));
    assert_eq!(state.gpr[7], 0x77, "the second load wrote nothing");
}

/// The first load writes its own base. When the second load faults, the
/// split rerun reads the first base again, so the pair writes no register
/// until both loads pass.
#[test]
fn a_first_load_into_its_own_base_reruns_from_the_old_base() {
    let code = [lwz(5, 5, 0), lwz(7, 6, 0x7FF0)];
    assert_fuses(&code, 0);
    let (fused, state) = run(&code, FLAT_END - 8, true, 256);
    assert_eq!(fused, run(&code, FLAT_END - 8, false, 256).0);
    assert_eq!(fused.reason, YieldReason::Fault);
    assert_eq!(
        (fused.pc, fused.faulting_ea),
        (Some(4), Some(0x66 + 0x7FF0))
    );
    assert_eq!(state.gpr[5], u64::from(LAST_WORDS[0]));
}

#[test]
fn two_loads_into_one_register_leave_the_second_value() {
    let code = [lwz(6, 5, 0), lwz(6, 5, 4)];
    assert_fuses(&code, 0);
    let (fused, state) = run(&code, FLAT_END - 8, true, 2);
    assert_eq!(fused, run(&code, FLAT_END - 8, false, 2).0);
    assert_eq!(state.gpr[6], u64::from(LAST_WORDS[1]));
}

#[test]
fn a_branch_into_the_second_load_runs_it_alone() {
    let code = [B_OVER_ONE, lwz(6, 5, 0), lwz(7, 5, 4)];
    assert_fuses(&code, 4);
    let (fused, state) = run(&code, FLAT_END - 8, true, 2);
    assert_eq!(fused, run(&code, FLAT_END - 8, false, 2).0);
    assert_eq!(state.gpr[6], 0x66, "the first load did not run");
    assert_eq!(state.gpr[7], u64::from(LAST_WORDS[1]));
}
