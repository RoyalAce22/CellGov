//! A fused `ld; rlwinm` stops where the two unfused instructions stop.

use crate::*;
use cellgov_event::UnitId;
use cellgov_exec::{ExecutionContext, ExecutionUnit, YieldReason};
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory, PageSize, Region};
use cellgov_time::Budget;

use crate::instruction::{FusedPair, PpuInstruction};
use crate::state::PpuState;

/// The end of the one region, which starts at 0.
const FLAT_END: u64 = 0x1000;
/// The doubleword at `FLAT_END - 8`.
const LAST_DOUBLEWORD: u64 = 0x0102_0304_0506_0708;

const B_OVER_ONE: u32 = (18 << 26) | 8;

/// `ld rT, d(r1)`.
fn ld_r1(rt: u32, d: i16) -> u32 {
    (58 << 26) | (rt << 21) | (1 << 16) | u32::from(d as u16 & !3)
}

/// `rlwinm rA, rS, sh, mb, me`, with Rc.
fn rlwinm(ra: u32, rs: u32, sh: u32, mb: u32, me: u32, rc: bool) -> u32 {
    (21 << 26) | (rs << 21) | (ra << 16) | (sh << 11) | (mb << 6) | (me << 1) | u32::from(rc)
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

/// Runs `code` at 0 with r1 = `r1`, fused or by plain decode, in batches of `budget`.
///
/// The run stops at the first of:
/// - a batch that faults
/// - a PC past the end of `code`
/// - the eighth batch
///
/// Returns the last batch's stop and the state it left.
fn run(code: &[u32], r1: u64, fused: bool, budget: u64) -> (Stop, PpuState) {
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
    mem.apply_commit(
        ByteRange::new(GuestAddr::new(FLAT_END - 8), 8).unwrap(),
        &LAST_DOUBLEWORD.to_be_bytes(),
    )
    .unwrap();
    let mut unit = PpuExecutionUnit::new(UnitId::new(0));
    if fused {
        unit.set_instruction_shadow(shadow::PredecodedShadow::build(0, &bytes));
    }
    unit.state_mut().set_gpr(1, r1);
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
            Some(PpuInstruction::Fused(FusedPair::LdRlwinm { .. }))
        ),
        "precondition: the ld and the rlwinm fuse at {at:#x}"
    );
}

/// [PPC-Book1 p:73 s:3.3.12] the rlwinm rotates the low word the load
/// just wrote, 0x05060708 by 8 to 0x06070805, and keeps bits 32..55:
/// 0x06070800. Rc = 1 sets CR0 to GT.
#[test]
fn the_rotate_reads_the_loaded_value() {
    let code = [ld_r1(6, -8), rlwinm(7, 6, 8, 0, 23, true)];
    assert_fuses(&code, 0);
    for budget in [1, 2, 256] {
        assert_eq!(
            run(&code, FLAT_END, true, budget).0,
            run(&code, FLAT_END, false, budget).0,
            "budget {budget}"
        );
    }
    let (_, state) = run(&code, FLAT_END, true, 256);
    assert_eq!(state.gpr[6], LAST_DOUBLEWORD);
    assert_eq!(state.gpr[7], 0x0607_0800);
    assert_eq!(state.cr_field(0), 0b0100);
}

/// [PPC-Book1 p:73 s:3.3.12] ROTL32 copies the rotated word into both
/// halves. With MB > ME, MASK(56, 39) is 0xFFFF_FFFF_FF00_00FF, so the
/// result keeps the high-word copy.
#[test]
fn a_wrapping_mask_keeps_high_word_bits() {
    let code = [ld_r1(6, -8), rlwinm(7, 6, 8, 24, 7, false)];
    assert_fuses(&code, 0);
    let (fused, state) = run(&code, FLAT_END, true, 256);
    assert_eq!(fused, run(&code, FLAT_END, false, 256).0);
    assert_eq!(state.gpr[7], 0x0607_0805_0600_0005);
}

#[test]
fn a_load_off_the_region_faults_with_nothing_written() {
    let code = [ld_r1(6, 0), rlwinm(7, 6, 8, 0, 23, false)];
    assert_fuses(&code, 0);
    let (fused, state) = run(&code, FLAT_END, true, 256);
    assert_eq!(fused, run(&code, FLAT_END, false, 256).0);
    assert_eq!(fused.reason, YieldReason::Fault);
    assert_eq!((fused.pc, fused.faulting_ea), (Some(0), Some(FLAT_END)));
    assert_eq!((state.gpr[6], state.gpr[7]), (0x66, 0x77));
}

#[test]
fn a_branch_into_the_rlwinm_runs_it_alone() {
    let code = [B_OVER_ONE, ld_r1(6, -8), rlwinm(7, 6, 4, 0, 27, false)];
    assert_fuses(&code, 4);
    let (fused, state) = run(&code, FLAT_END, true, 256);
    assert_eq!(fused, run(&code, FLAT_END, false, 256).0);
    assert_eq!(state.gpr[6], 0x66, "the ld did not run");
    assert_eq!(state.gpr[7], 0x660);
}
