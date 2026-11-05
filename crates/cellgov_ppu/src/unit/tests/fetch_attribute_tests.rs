//! Where the PPU refuses a fetch, a load and a store in a memory
//! restricted to its code spans and unmapped below its floor, and the
//! interrupt registers each refusal records.

use crate::shadow::PredecodedShadow;
use crate::*;
use cellgov_event::UnitId;
use cellgov_exec::{
    ExecutionContext, ExecutionStepResult, ExecutionUnit, InterruptRecord, YieldReason,
};
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory, PageSize, Region};
use cellgov_ps3_abi::hw::ppu_interrupt as irq;
use cellgov_time::Budget;

/// The floor: the PPU refuses every access below it.
const FLOOR: u64 = 0x1000;
/// The first code word.
const CODE: u64 = FLOOR;
/// A mapped word in the region at 0 that holds no code.
const DATA: u64 = 0x2000;
/// The base of a second region, which holds no code.
const STACK: u64 = 0x8000;

/// `li r3,1`.
const LI_R3_1: u32 = 0x3860_0001;
/// `blr`.
const BLR: u32 = 0x4E80_0020;

/// A memory whose code is `code` at [`CODE`], declared over its first
/// `declared` words, with `li r3,1; blr` at [`DATA`] and at [`STACK`].
fn memory(code: &[u32], declared: usize) -> GuestMemory {
    let mut mem = GuestMemory::from_regions(vec![
        Region::new(0, 0x4000, "main", PageSize::Page4K),
        Region::new(STACK, 0x1000, "stack", PageSize::Page4K),
    ])
    .unwrap();
    let mut put = |at: u64, word: u32| {
        let range = ByteRange::new(GuestAddr::new(at), 4).unwrap();
        mem.apply_commit(range, &word.to_be_bytes()).unwrap();
    };
    for (i, &word) in code.iter().enumerate() {
        put(CODE + 4 * i as u64, word);
    }
    for at in [DATA, STACK] {
        put(at, LI_R3_1);
        put(at + 4, BLR);
    }
    mem.declare_code(CODE, 4 * declared as u64);
    mem.unmap_below(FLOOR);
    mem.restrict_fetch();
    mem
}

/// Runs `unit` from `pc` in `mem` until it faults. The run commits nothing.
fn fault_from(mut unit: PpuExecutionUnit, mem: &GuestMemory, pc: u64) -> ExecutionStepResult {
    unit.state_mut().pc = pc;
    for _ in 0..8 {
        let r = unit.run_until_yield(Budget::new(8), &ExecutionContext::new(mem), &mut Vec::new());
        if r.yield_reason == YieldReason::Fault {
            return r;
        }
    }
    panic!("no fault from 0x{pc:x}");
}

/// The instruction storage record for a fetch at `srr0`.
fn fetch_refused(srr0: u64, srr1: u64) -> Option<InterruptRecord> {
    Some(InterruptRecord {
        vector: irq::VECTOR_INSTRUCTION_STORAGE,
        srr0,
        srr1,
        dsisr: None,
        dar: None,
    })
}

/// `mtctr r6; bctrl` with r6 = `target`.
fn call(target: u64) -> ExecutionStepResult {
    let mem = memory(&[0x7CC9_03A6, 0x4E80_0421], 2);
    let mut unit = PpuExecutionUnit::new(UnitId::new(0));
    unit.state_mut().set_gpr(6, target);
    fault_from(unit, &mem, CODE)
}

/// [PPC-Book3 p:68 s:5.5.5] SRR0 holds the branch target, and SRR1 bit 35 records a fetch from no-execute storage.
#[test]
fn a_call_into_mapped_data_takes_the_instruction_storage_interrupt_as_no_execute() {
    let r = call(DATA);
    assert_eq!(
        r.local_diagnostics.interrupt,
        fetch_refused(DATA, irq::SRR1_FETCH_NO_EXECUTE)
    );
    assert_eq!(
        r.fault,
        Some(cellgov_effects::FaultKind::Guest(
            FAULT_INSTRUCTION_STORAGE | (irq::SRR1_FETCH_NO_EXECUTE >> 16) as u32
        ))
    );
}

#[test]
fn a_call_into_another_mapped_region_is_refused_as_no_execute() {
    let r = call(STACK);
    assert_eq!(
        r.local_diagnostics.interrupt,
        fetch_refused(STACK, irq::SRR1_FETCH_NO_EXECUTE)
    );
}

/// [PPC-Book3 p:68 s:5.5.5] SRR1 bit 33 records a fetch that found no translation.
#[test]
fn a_call_below_the_floor_is_refused_as_no_translation() {
    let r = call(0);
    assert_eq!(
        r.local_diagnostics.interrupt,
        fetch_refused(0, irq::SRR1_FETCH_NOT_FOUND)
    );
}

#[test]
fn a_call_into_declared_code_runs() {
    let mut mem = memory(&[0x7CC9_03A6, 0x4E80_0421], 2);
    mem.declare_code(DATA, 8);
    let mut unit = PpuExecutionUnit::new(UnitId::new(0));
    unit.state_mut().set_gpr(6, DATA);
    // The return lands past the two code words.
    let r = fault_from(unit, &mem, CODE);
    assert_eq!(
        r.local_diagnostics.interrupt,
        fetch_refused(CODE + 8, irq::SRR1_FETCH_NO_EXECUTE)
    );
}

/// [PPC-Book3 p:67 s:5.5.3] DSISR bit 1 records a load that found no translation; DAR holds its EA.
#[test]
fn a_load_below_the_floor_takes_the_data_storage_interrupt() {
    let lwz_r4_0_r6 = (32 << 26) | (4 << 21) | (6 << 16);
    let mem = memory(&[lwz_r4_0_r6], 1);
    let mut unit = PpuExecutionUnit::new(UnitId::new(0));
    unit.state_mut().set_gpr(6, 0x10);
    let r = fault_from(unit, &mem, CODE);
    assert_eq!(
        r.local_diagnostics.interrupt,
        Some(InterruptRecord {
            vector: irq::VECTOR_DATA_STORAGE,
            srr0: CODE,
            srr1: 0,
            dsisr: Some(irq::DSISR_NOT_FOUND),
            dar: Some(0x10),
        })
    );
}

#[test]
fn a_store_into_declared_code_completes() {
    let stw_r5_0_r6 = (36 << 26) | (5 << 21) | (6 << 16);
    let mem = memory(&[stw_r5_0_r6], 1);
    let mut unit = PpuExecutionUnit::new(UnitId::new(0));
    unit.state_mut().set_gpr(6, CODE);
    let r = fault_from(unit, &mem, CODE);
    // The store retired; the fetch after it is the first refusal.
    assert_eq!(
        r.local_diagnostics.interrupt,
        fetch_refused(CODE + 4, irq::SRR1_FETCH_NO_EXECUTE)
    );
}

#[test]
fn a_fused_pair_whose_second_slot_is_not_code_runs_split() {
    let li_r3_5 = 0x3860_0005;
    // The store's target is mapped, so only the fetch refuses the slot.
    let stw_r3_data_r0 = (36 << 26) | (3 << 21) | DATA as u32;
    let mem = memory(&[li_r3_5, stw_r3_data_r0], 1);
    let shadow = PredecodedShadow::build(0, &mem.as_bytes()[..(CODE + 8) as usize]);
    assert!(
        shadow.get(CODE).is_some_and(|insn| insn.is_super_pair()),
        "the shadow fuses the pair"
    );
    let mut unit = PpuExecutionUnit::new(UnitId::new(0));
    unit.set_instruction_shadow(shadow);
    let r = fault_from(unit, &mem, CODE);
    assert_eq!(
        r.local_diagnostics.interrupt,
        fetch_refused(CODE + 4, irq::SRR1_FETCH_NO_EXECUTE)
    );
}
