//! The load-path probe and the load profile of a unit in profile mode.

use crate::profile::{LoadOutcome, LoadSummary};
use crate::*;
use cellgov_event::UnitId;
use cellgov_exec::{ExecutionContext, ExecutionUnit, YieldReason};
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory, PageSize, Region};
use cellgov_time::Budget;

/// Base of the data region; the text region is `[0, 0x1000)`.
const DATA: u64 = 0x1_0000;

/// `lwz r3, 0(r5)`.
const LWZ_R3_R5: u32 = (32 << 26) | (3 << 21) | (5 << 16);
/// `cmpwi cr0, r3, 0`.
const CMPWI_R3_0: u32 = (11 << 26) | (3 << 16);
/// `lwz r6, 0(r7)`.
const LWZ_R6_R7: u32 = (32 << 26) | (6 << 21) | (7 << 16);
/// `stw r8, 0x10(r5)`.
const STW_R8_R5: u32 = (36 << 26) | (8 << 21) | (5 << 16) | 0x10;
/// `lwz r9, 0x10(r5)`.
const LWZ_R9_R5: u32 = (32 << 26) | (9 << 21) | (5 << 16) | 0x10;
/// `xor r7, r7, r10`.
const XOR_R7_R10: u32 = (31 << 26) | (7 << 21) | (7 << 16) | (10 << 11) | (316 << 1);
/// `addi r5, r5, 4`.
const ADDI_R5_4: u32 = (14 << 26) | (5 << 21) | (5 << 16) | 4;
/// `bdnz -0x1c`: BO 16, back to 0.
const BDNZ_TO_0: u32 = (16 << 26) | (16 << 21) | 0xFFE4;
/// `sc`.
const SC: u32 = 0x4400_0002;

/// Three passes of a loop with three keyed loads:
///
/// - slot 0: `lwz; cmpwi`, fused, from the data region each pass
/// - slot 8: `lwz r6, 0(r7)`, with `r7` swapping between the text and
///   data regions each pass
/// - slot 0x10: `lwz r9`, which reads the word `stw r8` just stored
const PROGRAM: [u32; 9] = [
    LWZ_R3_R5, CMPWI_R3_0, LWZ_R6_R7, STW_R8_R5, LWZ_R9_R5, XOR_R7_R10, ADDI_R5_4, BDNZ_TO_0, SC,
];

fn memory() -> GuestMemory {
    let mut mem = GuestMemory::from_regions(vec![
        Region::new(0, 0x1000, "text", PageSize::Page4K),
        Region::new(DATA, 0x1000, "data", PageSize::Page4K),
    ])
    .unwrap();
    for (i, word) in PROGRAM.iter().enumerate() {
        let range = ByteRange::new(GuestAddr::new(4 * i as u64), 4).unwrap();
        mem.apply_commit(range, &word.to_be_bytes()).unwrap();
    }
    mem
}

fn unit(shadowed: bool, profiled: bool) -> PpuExecutionUnit {
    let bytes: Vec<u8> = PROGRAM.iter().flat_map(|w| w.to_be_bytes()).collect();
    let mut unit = PpuExecutionUnit::new(UnitId::new(0));
    if shadowed {
        unit.set_instruction_shadow(shadow::PredecodedShadow::build(0, &bytes));
    }
    let s = unit.state_mut();
    s.set_gpr(5, DATA);
    s.set_gpr(7, 0x800);
    s.set_gpr(10, 0x800 ^ (DATA + 0x800));
    s.set_ctr(3);
    unit.set_profile_mode(profiled);
    unit
}

/// Runs to the `sc` and returns the unit and its per-step hashes.
fn run(shadowed: bool, profiled: bool, traced: bool) -> (PpuExecutionUnit, Vec<(u64, u64)>) {
    let mem = memory();
    let mut unit = unit(shadowed, profiled);
    let ctx = ExecutionContext::new(&mem).with_trace_per_step(traced);
    let r = unit.run_until_yield(Budget::new(64), &ctx, &mut Vec::new());
    assert_eq!(r.yield_reason, YieldReason::Syscall);
    let hashes = unit.drain_retired_state_hashes();
    (unit, hashes)
}

#[test]
fn each_keyed_load_counts_once_by_its_slot() {
    let fused = unit(true, true).instruction_shadow().and_then(|s| s.get(0));
    assert!(
        fused.is_some_and(|insn| insn.is_super_pair()),
        "precondition: lwz; cmpwi fuses, got {fused:?}"
    );
    let (unit, _) = run(true, true, false);
    let profile = unit.load_profile();
    let sites: Vec<_> = profile.sites().collect();
    assert_eq!(sites, vec![(0, 3), (8, 3)]);
    assert_eq!(
        profile.summary(),
        LoadSummary {
            forwarded: 3,
            region_loads: 6,
            same_as_previous: 2,
            sites: 2,
            sites_by_regions: [1, 1, 0],
            loads_by_regions: [3, 3, 0],
            ..LoadSummary::default()
        }
    );
}

#[test]
fn a_unit_with_no_shadow_counts_its_region_loads_as_unslotted() {
    let (unit, _) = run(false, true, false);
    let s = unit.load_profile().summary();
    assert_eq!((s.no_shadow, s.forwarded, s.sites), (6, 3, 0));
}

#[test]
fn an_unprofiled_unit_counts_nothing() {
    let (unit, _) = run(true, false, false);
    assert_eq!(unit.load_profile().summary(), LoadSummary::default());
}

#[test]
fn profiling_leaves_the_state_hash_trace_alone() {
    let (plain, plain_trace) = run(true, false, true);
    let (profiled, profiled_trace) = run(true, true, true);
    assert_eq!(plain_trace.len(), 3 * 8 + 1);
    assert_eq!(profiled_trace, plain_trace);
    assert_eq!(profiled.state().fingerprint(), plain.state().fingerprint());
}

#[test]
fn a_load_past_the_top_of_the_address_space_is_unheld_and_faults_under_profiling() {
    // `lwz r3, -4(0)`: rA = 0 is literal zero, so EA is 2^64 - 4 and
    // the word runs past the top of the address space.
    const LWZ_R3_NEG4_R0: u32 = (32 << 26) | (3 << 21) | 0xFFFC;
    let mut mem = memory();
    let range = ByteRange::new(GuestAddr::new(0), 4).unwrap();
    mem.apply_commit(range, &LWZ_R3_NEG4_R0.to_be_bytes())
        .unwrap();
    let mut unit = PpuExecutionUnit::new(UnitId::new(0));
    unit.set_profile_mode(true);
    let ctx = ExecutionContext::new(&mem);
    let r = unit.run_until_yield(Budget::new(1), &ctx, &mut Vec::new());
    assert_eq!(r.yield_reason, YieldReason::Fault);
    assert_eq!(unit.load_profile().summary(), LoadSummary::default());
    // A faulted load never retires, so the empty summary says nothing
    // about the probe. The assert below checks the probe's own answer.
    let insn = decode::decode(LWZ_R3_NEG4_R0).unwrap();
    let views: Vec<_> = mem.region_views().collect();
    assert_eq!(
        unit.probe_load(&insn, 0, &views),
        Some((None, LoadOutcome::Unheld))
    );
}
