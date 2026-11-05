//! Each producer of a PPE interrupt yields the interrupt registers the
//! PPE sets: the vector, SRR0, the SRR1 cause bits, DSISR and DAR.

use crate::*;
use cellgov_event::UnitId;
use cellgov_exec::{
    DataAccessFault, ExecutionContext, ExecutionStepResult, ExecutionUnit, InterruptRecord,
    YieldReason,
};
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory, PageSize, Region, RegionAccess};
use cellgov_ps3_abi::hw::ppu_interrupt as irq;
use cellgov_time::Budget;

/// The end of the one writable region, which starts at 0.
const FLAT_END: u64 = 0x1000;
/// An address no region holds.
const UNMAPPED: u64 = 0x8000;
/// The base of a region that refuses writes.
const RSX: u64 = 0x1_0000;

fn memory(code: &[u32]) -> GuestMemory {
    let mut mem = GuestMemory::from_regions(vec![
        Region::new(0, FLAT_END as usize, "flat", PageSize::Page4K),
        Region::with_access(
            RSX,
            0x1000,
            "rsx",
            PageSize::Page4K,
            RegionAccess::ReservedZeroReadable,
        ),
    ])
    .unwrap();
    for (i, word) in code.iter().enumerate() {
        let range = ByteRange::new(GuestAddr::new(4 * i as u64), 4).unwrap();
        mem.apply_commit(range, &word.to_be_bytes()).unwrap();
    }
    mem
}

/// Runs `insn` at 0 with r5 = `r5` until the unit yields.
fn fault_of(insn: u32, r5: u64) -> ExecutionStepResult {
    let mem = memory(&[insn]);
    let mut unit = PpuExecutionUnit::new(UnitId::new(0));
    unit.state_mut().set_gpr(5, r5);
    let r = unit.run_until_yield(
        Budget::new(8),
        &ExecutionContext::new(&mem),
        &mut Vec::new(),
    );
    assert_eq!(r.yield_reason, YieldReason::Fault, "0x{insn:08x}: {r:?}");
    r
}

fn record(
    vector: u64,
    srr0: u64,
    srr1: u64,
    dsisr: Option<u32>,
    dar: Option<u64>,
) -> InterruptRecord {
    InterruptRecord {
        vector,
        srr0,
        srr1,
        dsisr,
        dar,
    }
}

/// `op r6, 0(r5)`.
fn d_form(op: u32) -> u32 {
    (op << 26) | (6 << 21) | (5 << 16)
}

/// [PPC-Book3 p:68 s:5.5.5] SRR0 holds the address the fetch failed at, and SRR1 bit 33 records no translation found.
#[test]
fn a_fetch_past_guest_memory_takes_the_instruction_storage_interrupt() {
    let mem = memory(&[]);
    let mut unit = PpuExecutionUnit::new(UnitId::new(0));
    unit.state_mut().pc = UNMAPPED;
    let r = unit.run_until_yield(
        Budget::new(8),
        &ExecutionContext::new(&mem),
        &mut Vec::new(),
    );
    assert_eq!(
        r.local_diagnostics.interrupt,
        Some(record(
            irq::VECTOR_INSTRUCTION_STORAGE,
            UNMAPPED,
            irq::SRR1_FETCH_NOT_FOUND,
            None,
            None
        ))
    );
}

/// [PPC-Book3 p:67 s:5.5.3] SRR0 holds the load; DSISR bit 1 records no translation found; DAR holds the EA.
#[test]
fn a_load_from_an_unmapped_address_takes_the_data_storage_interrupt() {
    let r = fault_of(d_form(32), UNMAPPED);
    assert_eq!(
        r.local_diagnostics.interrupt,
        Some(record(
            irq::VECTOR_DATA_STORAGE,
            0,
            0,
            Some(irq::DSISR_NOT_FOUND),
            Some(UNMAPPED)
        ))
    );
    assert_eq!(r.local_diagnostics.faulting_ea, Some(UNMAPPED));
}

/// [PPC-Book3 p:67 s:5.5.3] DSISR bit 6 marks a store.
#[test]
fn a_store_to_an_unmapped_address_sets_the_store_bit() {
    let r = fault_of(d_form(36), UNMAPPED);
    assert_eq!(
        r.local_diagnostics.interrupt,
        Some(record(
            irq::VECTOR_DATA_STORAGE,
            0,
            0,
            Some(irq::DSISR_NOT_FOUND | irq::DSISR_STORE),
            Some(UNMAPPED)
        ))
    );
}

/// [PPC-Book3 p:67 s:5.5.3] DSISR bit 4 records a storage protection refusal.
#[test]
fn a_store_a_region_refuses_records_a_protection_refusal() {
    let r = fault_of(d_form(36), RSX);
    assert_eq!(
        r.local_diagnostics.interrupt,
        Some(record(
            irq::VECTOR_DATA_STORAGE,
            0,
            0,
            Some(irq::DSISR_PROTECTION | irq::DSISR_STORE),
            Some(RSX)
        ))
    );
    assert_eq!(
        r.local_diagnostics.data_access,
        Some(DataAccessFault {
            store: true,
            refused_by: Some("rsx")
        })
    );
}

/// [PPC-Book3 p:70 s:5.5.8] DAR holds the EA. [CBE-Handbook p:253 s:9.5.8] The PPE leaves DSISR alone.
#[test]
fn a_misaligned_lwarx_takes_the_alignment_interrupt() {
    let lwarx_r6_0_r5 = (31 << 26) | (6 << 21) | (5 << 11) | (20 << 1);
    let r = fault_of(lwarx_r6_0_r5, 0x101);
    assert_eq!(
        r.local_diagnostics.interrupt,
        Some(record(irq::VECTOR_ALIGNMENT, 0, 0, None, Some(0x101)))
    );
    assert_eq!(
        r.local_diagnostics.data_access,
        Some(DataAccessFault {
            store: false,
            refused_by: None
        })
    );
}

/// [PPC-Book3 p:72 s:5.5.9] SRR1 bit 44 marks an illegal instruction.
#[test]
fn a_word_with_no_decode_takes_the_illegal_instruction_program_interrupt() {
    let r = fault_of(0, 0);
    assert_eq!(
        r.local_diagnostics.interrupt,
        Some(record(
            irq::VECTOR_PROGRAM,
            0,
            irq::SRR1_PROGRAM_ILLEGAL,
            None,
            None
        ))
    );
}

/// [PPC-Book3 p:72 s:5.5.9] SRR1 bit 45 marks a privileged instruction.
#[test]
fn mfmsr_takes_the_privileged_instruction_program_interrupt() {
    let r = fault_of(0x7C60_00A6, 0);
    assert_eq!(
        r.local_diagnostics.interrupt,
        Some(record(
            irq::VECTOR_PROGRAM,
            0,
            irq::SRR1_PROGRAM_PRIVILEGED,
            None,
            None
        ))
    );
}

/// [PPC-Book3 p:72 s:5.5.9] SRR1 bit 46 marks a trap.
#[test]
fn a_fired_tw_takes_the_trap_program_interrupt() {
    let tw_31_r0_r0 = (31 << 26) | (31 << 21) | (4 << 1);
    let r = fault_of(tw_31_r0_r0, 0);
    assert_eq!(
        r.local_diagnostics.interrupt,
        Some(record(
            irq::VECTOR_PROGRAM,
            0,
            irq::SRR1_PROGRAM_TRAP,
            None,
            None
        ))
    );
}

/// [PPC-Book3 p:12 s:2.3.1] A system call interrupt sets SRR0 to the instruction after `sc`.
#[test]
fn sc_one_takes_the_system_call_interrupt_with_srr0_after_it() {
    let r = fault_of(0x4400_0022, 0);
    assert_eq!(
        r.local_diagnostics.interrupt,
        Some(record(irq::VECTOR_SYSTEM_CALL, 4, 0, None, None))
    );
    assert_eq!(r.local_diagnostics.pc, Some(0));
}

#[test]
fn a_debug_break_is_no_interrupt() {
    let mem = memory(&[(14 << 26) | (3 << 21) | 1]);
    let mut unit = PpuExecutionUnit::new(UnitId::new(0));
    unit.set_break_pc(0, 0);
    let r = unit.run_until_yield(
        Budget::new(8),
        &ExecutionContext::new(&mem),
        &mut Vec::new(),
    );
    assert_eq!(
        r.fault,
        Some(cellgov_effects::FaultKind::Guest(FAULT_DEBUG_BREAK))
    );
    assert_eq!(r.local_diagnostics.interrupt, None);
    assert_eq!(r.local_diagnostics.data_access, None);
}
