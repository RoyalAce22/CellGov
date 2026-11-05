//! A fetch from a mapped region other than the one at base 0 reads that
//! region's code, and the interrupt a refused fetch records there.

use crate::*;
use cellgov_event::UnitId;
use cellgov_exec::{
    ExecutionContext, ExecutionStepResult, ExecutionUnit, InterruptRecord, YieldReason,
};
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory, PageSize, Region, RegionAccess};
use cellgov_ps3_abi::hw::ppu_interrupt as irq;
use cellgov_time::Budget;

/// The base of the second read-write region.
const STACK: u64 = 0x8000;
/// The base of a zero-readable reserved region.
const RESERVED: u64 = 0x1_0000;
/// Where the `blr` at [`STACK`] returns: a zero word, which has no decode.
const RETURN: u64 = STACK + 0x100;

/// `li r3,1`.
const LI_R3_1: u32 = 0x3860_0001;
/// `blr`.
const BLR: u32 = 0x4E80_0020;
/// `b +0x8000`: from address 0 to [`STACK`].
const B_TO_STACK: u32 = 0x4800_8000;

/// A memory with `li r3,1; blr` at [`STACK`] and a zero-readable region
/// at [`RESERVED`].
fn memory() -> GuestMemory {
    let mut mem = GuestMemory::from_regions(vec![
        Region::new(0, 0x4000, "main", PageSize::Page4K),
        Region::new(STACK, 0x1000, "stack", PageSize::Page4K),
        Region::with_access(
            RESERVED,
            0x1000,
            "reserved",
            PageSize::Page4K,
            RegionAccess::ReservedZeroReadable,
        ),
    ])
    .unwrap();
    for (at, word) in [(STACK, LI_R3_1), (STACK + 4, BLR)] {
        let range = ByteRange::new(GuestAddr::new(at), 4).unwrap();
        mem.apply_commit(range, &word.to_be_bytes()).unwrap();
    }
    mem
}

/// Runs a unit from `pc` with LR = [`RETURN`] until it faults.
fn fault_from(mem: &GuestMemory, pc: u64) -> (ExecutionStepResult, PpuExecutionUnit) {
    let mut unit = PpuExecutionUnit::new(UnitId::new(0));
    unit.state_mut().pc = pc;
    unit.state_mut().set_lr(RETURN);
    for _ in 0..8 {
        let r = unit.run_until_yield(Budget::new(8), &ExecutionContext::new(mem), &mut Vec::new());
        if r.yield_reason == YieldReason::Fault {
            return (r, unit);
        }
    }
    panic!("no fault from 0x{pc:x}");
}

/// [PPC-Book3 p:72 s:5.5.9] SRR1 bit 44 records an illegal instruction, so the two words at the stack ran.
fn assert_ran_the_stack_code(mem: &GuestMemory) {
    let (r, unit) = fault_from(mem, STACK);
    assert_eq!(unit.state().gpr[3], 1);
    assert_eq!(
        r.local_diagnostics.interrupt,
        Some(InterruptRecord {
            vector: irq::VECTOR_PROGRAM,
            srr0: RETURN,
            srr1: irq::SRR1_PROGRAM_ILLEGAL,
            dsisr: None,
            dar: None,
        })
    );
}

#[test]
fn an_unrestricted_memory_runs_code_in_a_region_other_than_the_one_at_zero() {
    assert_ran_the_stack_code(&memory());
}

#[test]
fn a_restricted_memory_runs_a_code_span_declared_in_another_region() {
    let mut mem = memory();
    mem.declare_code(STACK, 0x1000);
    mem.restrict_fetch();
    assert_ran_the_stack_code(&mem);
}

#[test]
fn one_batch_that_branches_into_another_region_runs_that_regions_code() {
    let mut mem = memory();
    let range = ByteRange::new(GuestAddr::new(0), 4).unwrap();
    mem.apply_commit(range, &B_TO_STACK.to_be_bytes()).unwrap();
    let mut unit = PpuExecutionUnit::new(UnitId::new(0));
    unit.state_mut().pc = 0;
    unit.state_mut().set_lr(RETURN);
    let r = unit.run_until_yield(
        Budget::new(8),
        &ExecutionContext::new(&mem),
        &mut Vec::new(),
    );
    assert_eq!(r.local_diagnostics.interrupt, None);
    assert_eq!(unit.state().gpr[3], 1);
}

/// [PPC-Book3 p:68 s:5.5.5] SRR1 bit 35 records a fetch from mapped storage the fetch cannot run.
#[test]
fn a_fetch_from_a_zero_reading_region_is_refused_as_no_execute() {
    let (r, _) = fault_from(&memory(), RESERVED);
    assert_eq!(
        r.local_diagnostics.interrupt,
        Some(InterruptRecord {
            vector: irq::VECTOR_INSTRUCTION_STORAGE,
            srr0: RESERVED,
            srr1: irq::SRR1_FETCH_NO_EXECUTE,
            dsisr: None,
            dar: None,
        })
    );
}
