//! Profile mode counts every instruction a unit retires, with both halves
//! of a fused pair, and leaves the run unchanged.

use crate::profile::{InsnClass, PairClass};
use crate::*;
use cellgov_event::UnitId;
use cellgov_exec::{ExecutionContext, ExecutionUnit, YieldReason};
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_time::Budget;

/// `lwz r3, 0(r5)`.
const LWZ_R3_R5: u32 = (32 << 26) | (3 << 21) | (5 << 16);
/// `cmpwi cr0, r3, 0`.
const CMPWI_R3_0: u32 = (11 << 26) | (3 << 16);
/// `beq +8`: BO 12, BI 2.
const BEQ_8: u32 = (16 << 26) | (12 << 21) | (2 << 16) | 8;
/// `addi r4, r4, 1`.
const ADDI_R4: u32 = (14 << 26) | (4 << 21) | (4 << 16) | 1;
/// `addi r6, r6, 1`.
const ADDI_R6: u32 = (14 << 26) | (6 << 21) | (6 << 16) | 1;
/// `sc`.
const SC: u32 = 0x4400_0002;

/// `lwz; cmpwi; beq` over a skipped `addi`, then an `addi` and an `sc`.
/// The shadow fuses `lwz; cmpwi`, which hides `cmpwi; beq` from a count
/// of dispatched variants.
const PROGRAM: [u32; 6] = [LWZ_R3_R5, CMPWI_R3_0, BEQ_8, ADDI_R4, ADDI_R6, SC];

fn memory() -> GuestMemory {
    let mut mem = GuestMemory::new(0x1000);
    for (i, word) in PROGRAM.iter().enumerate() {
        let range = ByteRange::new(GuestAddr::new(4 * i as u64), 4).unwrap();
        mem.apply_commit(range, &word.to_be_bytes()).unwrap();
    }
    mem
}

fn unit(profiled: bool) -> PpuExecutionUnit {
    let bytes: Vec<u8> = PROGRAM.iter().flat_map(|w| w.to_be_bytes()).collect();
    let mut unit = PpuExecutionUnit::new(UnitId::new(0));
    unit.set_instruction_shadow(shadow::PredecodedShadow::build(0, &bytes));
    unit.state_mut().set_gpr(5, 0x800);
    unit.set_profile_mode(profiled);
    unit
}

/// Runs to the `sc` and returns the per-step hashes and the final state.
fn run(profiled: bool, traced: bool) -> (PpuExecutionUnit, Vec<(u64, u64)>) {
    let mem = memory();
    let mut unit = unit(profiled);
    let ctx = ExecutionContext::new(&mem).with_trace_per_step(traced);
    let mut effects = Vec::new();
    let r = unit.run_until_yield(Budget::new(64), &ctx, &mut effects);
    assert_eq!(r.yield_reason, YieldReason::Syscall);
    let hashes = unit.drain_retired_state_hashes();
    (unit, hashes)
}

#[test]
fn every_retired_word_counts_once_with_the_pair_the_shadow_fused() {
    let fused = unit(true).instruction_shadow().and_then(|s| s.get(0));
    assert!(
        fused.is_some_and(|insn| insn.is_super_pair()),
        "precondition: lwz; cmpwi fuses, got {fused:?}"
    );

    let (unit, _) = run(true, false);
    let profile = unit.operand_profile();

    let pcs: Vec<_> = profile.pcs.iter().map(|(pc, n)| (*pc, *n)).collect();
    assert_eq!(pcs, vec![(0, 1), (4, 1), (8, 1), (16, 1), (20, 1)]);
    let (insns, pairs) = profile.classes(|pc| PROGRAM.get(usize::try_from(pc / 4).ok()?).copied());
    assert_eq!(insns.values().sum::<u64>(), 5);
    let hidden = PairClass::of(CMPWI_R3_0, BEQ_8).unwrap();
    assert_eq!(pairs.get(&hidden), Some(&1), "{pairs:?}");
    let fused_pair = PairClass::of(LWZ_R3_R5, CMPWI_R3_0).unwrap();
    assert_eq!(pairs.get(&fused_pair), Some(&1));
    assert_eq!(pairs.values().sum::<u64>(), 3, "8 -> 16 is a branch");
    let cmpwi = InsnClass::of(CMPWI_R3_0).unwrap();
    assert_eq!(insns.get(&cmpwi), Some(&1));
}

#[test]
fn a_fused_pair_whose_second_half_faults_counts_only_its_first_half() {
    use cellgov_mem::{PageSize, Region};
    /// `addi r3, r3, 1`.
    const ADDI_R3: u32 = (14 << 26) | (3 << 21) | (3 << 16) | 1;
    /// `li r6, 7`.
    const LI_R6_7: u32 = (14 << 26) | (6 << 21) | 7;
    /// `stw r6, 0(r5)`.
    const STW_R6_R5: u32 = (36 << 26) | (6 << 21) | (5 << 16);
    let code = [ADDI_R3, LI_R6_7, STW_R6_R5, ADDI_R3];
    let bytes: Vec<u8> = code.iter().flat_map(|w| w.to_be_bytes()).collect();
    let shadow = shadow::PredecodedShadow::build(0, &bytes);
    assert!(
        shadow.get(4).is_some_and(|insn| insn.is_super_pair()),
        "precondition: li; stw fuses at 4"
    );
    let mut mem =
        GuestMemory::from_regions(vec![Region::new(0, 0x1000, "flat", PageSize::Page4K)]).unwrap();
    for (i, word) in code.iter().enumerate() {
        let range = ByteRange::new(GuestAddr::new(4 * i as u64), 4).unwrap();
        mem.apply_commit(range, &word.to_be_bytes()).unwrap();
    }
    let mut unit = PpuExecutionUnit::new(UnitId::new(0));
    unit.set_instruction_shadow(shadow);
    // The store's target lies past the one region.
    unit.state_mut().set_gpr(5, 0x8000);
    unit.set_profile_mode(true);
    let ctx = ExecutionContext::new(&mem);
    let mut effects = Vec::new();
    let mut faulted = false;
    for _ in 0..4 {
        let r = unit.run_until_yield(Budget::new(4), &ctx, &mut effects);
        if r.yield_reason == YieldReason::Fault {
            faulted = true;
            break;
        }
    }
    assert!(faulted, "the stw faults");

    let pcs: Vec<_> = unit
        .operand_profile()
        .pcs
        .iter()
        .map(|(pc, n)| (*pc, *n))
        .collect();
    assert_eq!(
        pcs,
        vec![(0, 1), (4, 1)],
        "addi and li retired, stw did not"
    );
    assert_eq!(
        unit.operand_profile().retired(),
        unit.retired_instructions()
    );
}

#[test]
fn profiling_leaves_the_state_hash_trace_and_the_final_state_alone() {
    let (plain, plain_trace) = run(false, true);
    let (profiled, profiled_trace) = run(true, true);
    assert_eq!(plain_trace.len(), 5);
    assert_eq!(profiled_trace, plain_trace);
    assert_eq!(profiled.state().fingerprint(), plain.state().fingerprint());

    let (plain, _) = run(false, false);
    let (profiled, _) = run(true, false);
    assert_eq!(profiled.state().fingerprint(), plain.state().fingerprint());
    assert_eq!(
        profiled.retired_instructions(),
        plain.retired_instructions()
    );
}
