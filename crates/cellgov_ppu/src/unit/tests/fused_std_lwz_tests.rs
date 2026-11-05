//! A fused `std; lwz` stops where the two instructions stop unfused, and
//! emits the same store.
//!
//! [PPC-Book3 p:61 s:5.2] when an interrupt occurs, every preceding
//! instruction is complete, so a load that faults leaves the store before
//! it in place.

use crate::*;
use cellgov_effects::Effect;
use cellgov_event::UnitId;
use cellgov_exec::{ExecutionContext, ExecutionUnit, YieldReason};
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory, PageSize, Region};
use cellgov_time::Budget;

use crate::instruction::{FusedPair, PpuInstruction};
use crate::state::PpuState;

/// The end of the one region, which starts at 0.
const FLAT_END: u64 = 0x1000;
/// The stack pointer the runs start with.
const STACK: u64 = 0x800;
/// The words at `STACK + 16` and `STACK + 20` before any store.
const STACK_WORDS: [u32; 2] = [0x0102_0304, 0x0506_0708];
/// The doubleword r5 holds.
const STORED: u64 = 0x1122_3344_5566_7788;

const B_OVER_ONE: u32 = (18 << 26) | 8;

/// `std rS, ds(rA)`.
fn std(rs: u32, ra: u32, ds: i16) -> u32 {
    (62 << 26) | (rs << 21) | (ra << 16) | u32::from(ds as u16)
}

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
    writes: Vec<Effect>,
}

/// Runs `code` at 0 in batches of `budget`, fused or by plain decode.
///
/// The run starts with r1 = `r1`, r5 = [`STORED`] and r7 = `r7`. It stops
/// at the first of:
/// - a batch that faults
/// - a PC past the end of `code`
/// - the end of the eighth batch
///
/// Returns the last batch's stop with the stores of every batch, and the
/// state the run left.
fn run(code: &[u32], r1: u64, r7: u64, fused: bool, budget: u64) -> (Stop, PpuState) {
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
    let stack = STACK_WORDS
        .iter()
        .flat_map(|w| w.to_be_bytes())
        .collect::<Vec<_>>();
    mem.apply_commit(
        ByteRange::new(GuestAddr::new(STACK + 16), 8).unwrap(),
        &stack,
    )
    .unwrap();
    let mut unit = PpuExecutionUnit::new(UnitId::new(0));
    if fused {
        unit.set_instruction_shadow(shadow::PredecodedShadow::build(0, &bytes));
    }
    unit.state_mut().set_gpr(1, r1);
    unit.state_mut().set_gpr(5, STORED);
    unit.state_mut().set_gpr(6, 0x66);
    unit.state_mut().set_gpr(7, r7);
    let mut writes = Vec::new();
    let mut stop = None;
    for _ in 0..8 {
        let mut effects = Vec::new();
        let r = unit.run_until_yield(
            Budget::new(budget),
            &ExecutionContext::new(&mem),
            &mut effects,
        );
        writes.extend(
            effects
                .into_iter()
                .filter(|e| matches!(e, Effect::SharedWriteIntent { .. })),
        );
        let fault = r.yield_reason == YieldReason::Fault;
        stop = Some((
            r.yield_reason,
            r.local_diagnostics.pc,
            r.local_diagnostics.faulting_ea,
        ));
        if fault || unit.state().pc >= 4 * code.len() as u64 {
            break;
        }
    }
    let (reason, pc, faulting_ea) = stop.expect("one batch ran");
    let stop = Stop {
        reason,
        pc,
        faulting_ea,
        retired: unit.retired_instructions(),
        state: unit.state().fingerprint(),
        writes,
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
            Some(PpuInstruction::Fused(FusedPair::StdLwz { .. }))
        ),
        "precondition: the pair fuses at {at:#x}"
    );
}

/// [PPC-Book1 p:17 s:2.2] the lwz runs after the std, so it reads the
/// bytes the std wrote: the high word at offset 16, the low word at 20.
#[test]
fn the_load_reads_the_doubleword_the_store_wrote() {
    for (offset, word) in [(16, STORED >> 32), (20, STORED & 0xFFFF_FFFF)] {
        let code = [std(5, 1, 16), lwz(6, 1, offset)];
        assert_fuses(&code, 0);
        let (fused, state) = run(&code, STACK, 0, true, 256);
        assert_eq!(fused, run(&code, STACK, 0, false, 256).0, "offset {offset}");
        assert_eq!(state.gpr[6], word, "offset {offset}");
        assert_eq!(fused.writes.len(), 1, "offset {offset}");
    }
}

#[test]
fn a_load_off_the_region_faults_after_the_store_retires() {
    let code = [std(5, 1, 16), lwz(6, 7, 0)];
    assert_fuses(&code, 0);
    for budget in [1, 2, 256] {
        assert_eq!(
            run(&code, STACK, FLAT_END, true, budget).0,
            run(&code, STACK, FLAT_END, false, budget).0,
            "budget {budget}"
        );
    }
    let (stop, state) = run(&code, STACK, FLAT_END, true, 256);
    assert_eq!(stop.reason, YieldReason::Fault);
    assert_eq!(
        (stop.pc, stop.faulting_ea),
        (Some(4), Some(FLAT_END)),
        "the load faults at its own address"
    );
    assert_eq!(stop.retired, 1, "the store retired");
    assert_eq!(stop.writes.len(), 1, "the store is emitted once");
    assert_eq!(state.gpr[6], 0x66, "the load wrote nothing");
}

#[test]
fn a_store_off_the_region_faults_before_the_load_runs() {
    let code = [std(5, 1, 0), lwz(6, 7, 0)];
    assert_fuses(&code, 0);
    let (fused, state) = run(&code, FLAT_END, STACK + 16, true, 256);
    assert_eq!(fused, run(&code, FLAT_END, STACK + 16, false, 256).0);
    assert_eq!(fused.reason, YieldReason::Fault);
    assert_eq!((fused.pc, fused.faulting_ea), (Some(0), Some(FLAT_END)));
    assert_eq!(fused.retired, 0);
    assert!(fused.writes.is_empty());
    assert_eq!(state.gpr[6], 0x66, "the load did not run");
}

#[test]
fn a_branch_into_the_load_runs_it_alone() {
    let code = [B_OVER_ONE, std(5, 1, 16), lwz(6, 1, 16)];
    assert_fuses(&code, 4);
    let (fused, state) = run(&code, STACK, 0, true, 2);
    assert_eq!(fused, run(&code, STACK, 0, false, 2).0);
    assert!(fused.writes.is_empty(), "the store did not run");
    assert_eq!(state.gpr[6], u64::from(STACK_WORDS[0]));
}
