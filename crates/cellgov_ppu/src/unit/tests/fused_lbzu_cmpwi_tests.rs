//! A fused `lbzu; cmpwi 0` stops where the two instructions stop unfused.

use crate::*;
use cellgov_event::UnitId;
use cellgov_exec::{ExecutionContext, ExecutionUnit, YieldReason};
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory, PageSize, Region};
use cellgov_time::Budget;

use crate::instruction::{FusedPair, PpuInstruction};
use crate::state::PpuState;

/// The end of the one region, which starts at 0.
const FLAT_END: u64 = 0x1000;
/// Where the string `"abc\0"` starts.
const STRING: u64 = 0x100;

const B_OVER_ONE: u32 = (18 << 26) | 8;
/// `addi r3, r3, 1`.
const ADDI_R3: u32 = (14 << 26) | (3 << 21) | (3 << 16) | 1;
/// `lbzu r4, 1(r5)`.
const LBZU_R4: u32 = (35 << 26) | (4 << 21) | (5 << 16) | 1;
/// `cmpwi cr7, r4, 0`.
const CMPWI_CR7_R4: u32 = (11 << 26) | (7 << 23) | (4 << 16);
/// `bne cr7, -8`: BO 4 branches when CR bit 30 (cr7 EQ) is 0.
const BNE_CR7_BACK_TWO: u32 = (16 << 26) | (4 << 21) | (30 << 16) | (-8i16 as u16 as u32);

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

/// Runs `code` at 0 with r5 = `r5`, fused or by plain decode, in batches of `budget`.
///
/// The run stops at the first of:
/// - a batch that faults
/// - a PC past the end of `code`
/// - the sixty-fourth batch
///
/// Returns the last batch's stop and the state it left.
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
    mem.apply_commit(ByteRange::new(GuestAddr::new(STRING), 4).unwrap(), b"abc\0")
        .unwrap();
    let mut unit = PpuExecutionUnit::new(UnitId::new(0));
    if fused {
        unit.set_instruction_shadow(shadow::PredecodedShadow::build(0, &bytes));
    }
    unit.state_mut().set_gpr(4, 0x44);
    unit.state_mut().set_gpr(5, r5);
    let mut stop = None;
    for _ in 0..64 {
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

fn shadow_of(code: &[u32]) -> shadow::PredecodedShadow {
    shadow::PredecodedShadow::build(
        0,
        &code
            .iter()
            .flat_map(|w| w.to_be_bytes())
            .collect::<Vec<_>>(),
    )
}

#[test]
fn a_string_scan_stops_on_the_terminating_zero() {
    let code = [LBZU_R4, CMPWI_CR7_R4, BNE_CR7_BACK_TWO, ADDI_R3];
    assert!(
        matches!(
            shadow_of(&code).get(0),
            Some(PpuInstruction::Fused(FusedPair::LbzuCmpwi { .. }))
        ),
        "precondition: the lbzu and the cmpwi fuse"
    );
    for budget in [1, 2, 3, 256] {
        assert_eq!(
            run(&code, STRING - 1, true, budget).0,
            run(&code, STRING - 1, false, budget).0,
            "budget {budget}"
        );
    }
    let (_, state) = run(&code, STRING - 1, true, 256);
    assert_eq!(state.gpr[5], STRING + 3, "r5 points at the zero byte");
    assert_eq!(state.gpr[4], 0);
    assert_eq!(state.gpr[3], 1, "the loop fell through once");
    assert_eq!(state.cr_field(7), 0b0010, "cr7 is EQ");
}

/// [PPC-Book1 p:34 s:3.3.2] lbzu loads RT, then writes EA to RA.
/// [PPC-Book2 p:16 s:2.1] an aborted load keeps the registers its re-execution
/// reads, so a faulting lbzu leaves r5 as it was. r4 holds because no load landed.
#[test]
fn a_load_off_the_region_faults_with_ra_unchanged() {
    let code = [ADDI_R3, LBZU_R4, CMPWI_CR7_R4, ADDI_R3];
    assert!(
        matches!(
            shadow_of(&code).get(4),
            Some(PpuInstruction::Fused(FusedPair::LbzuCmpwi { .. }))
        ),
        "precondition: the lbzu and the cmpwi fuse"
    );
    for budget in [1, 2, 3, 256] {
        assert_eq!(
            run(&code, FLAT_END - 1, true, budget).0,
            run(&code, FLAT_END - 1, false, budget).0,
            "budget {budget}"
        );
    }
    let (stop, state) = run(&code, FLAT_END - 1, true, 256);
    assert_eq!(stop.reason, YieldReason::Fault);
    assert_eq!(stop.pc, Some(4));
    assert_eq!(stop.faulting_ea, Some(FLAT_END));
    assert_eq!(state.gpr[5], FLAT_END - 1);
    assert_eq!(state.gpr[4], 0x44);
}

#[test]
fn a_branch_into_the_compare_runs_it_alone() {
    let code = [B_OVER_ONE, LBZU_R4, CMPWI_CR7_R4];
    assert_eq!(
        shadow_of(&code).get(8),
        Some(PpuInstruction::Consumed),
        "precondition: the compare is the second half of a fused pair"
    );
    let (fused, state) = run(&code, STRING - 1, true, 2);
    assert_eq!(fused, run(&code, STRING - 1, false, 2).0);
    assert_eq!(state.gpr[5], STRING - 1, "the lbzu did not run");
    assert_eq!(state.cr_field(7), 0b0100, "cr7 is GT for r4 = 0x44");
}
