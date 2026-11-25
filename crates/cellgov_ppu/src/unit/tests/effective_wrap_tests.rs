//! A translating thread's access to the last page of the 64-bit effective
//! space reaches its storage, and an access that runs past 2^64 - 1
//! wraps to the page at 0.
//!
//! [PPC-Book1 p:15 s:1.12.2] In 64-bit mode the effective address arithmetic wraps from 2^64 - 1 to 0.

use crate::mmu::Partition;
use crate::supervisor::{SlbEntry, SupervisorState};
use crate::*;
use cellgov_effects::Effect;
use cellgov_event::UnitId;
use cellgov_exec::{ExecutionContext, ExecutionStepResult, ExecutionUnit, YieldReason};
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_ps3_abi::hw::ppu_interrupt as irq;
use cellgov_ps3_abi::hw::ppu_mmu::{PTE_C, PTE_I, PTE_R};
use cellgov_ps3_abi::hw::ppu_supervisor::{MSR_DR, MSR_IR, MSR_ME, MSR_SF, SLB_VALID};
use cellgov_time::Budget;

/// The page table: 2^11 groups from 256 KB to the end of the memory.
const HTAB: u64 = 0x4_0000;
/// The code segment, its VSID, and the real address of its page 0.
const CODE_EA: u64 = 0xC000_0000;
const CODE_VSID: u64 = 0x123;
const CODE_RA: u64 = 0x3000;
/// The last page of the effective space, and its real address. It is
/// page 0xFFFF of the last segment, which uses [`CODE_VSID`].
const TOP_PAGE: u64 = 0xFFFF_FFFF_FFFF_F000;
const TOP_RA: u64 = 0x7000;
/// The VSID of segment 0, and the real address of its page 0, the page
/// at EA 0.
const LOW_VSID: u64 = 0x456;
const LOW_RA: u64 = 0x8000;
/// A real address past the 512 KB the memory holds.
const UNBACKED_RA: u64 = 0x9_0000;
/// Read/write with R and C set, so no access writes the page table.
const RW: u64 = 0b10 | PTE_R | PTE_C;

fn put(mem: &mut GuestMemory, at: u64, bytes: &[u8]) {
    let range = ByteRange::new(GuestAddr::new(at), bytes.len() as u64).unwrap();
    mem.apply_commit(range, bytes).unwrap();
}

/// Writes the entry for page `index` of the segment with `vsid` to slot 0
/// of its primary group.
///
/// The hash is the VSID XOR the page index. The AVPN holds the VSID and
/// the five high bits of the page index.
fn map(mem: &mut GuestMemory, vsid: u64, index: u64, d1: u64) {
    let pteg = HTAB + ((vsid ^ index) & 0x7FF) * 128;
    let d0 = (vsid << 12) | ((index >> 11) << 7) | 1;
    put(mem, pteg, &d0.to_be_bytes());
    put(mem, pteg + 8, &d1.to_be_bytes());
}

/// 512 KB of memory that holds `code` on the code page and maps:
///
/// - the code page;
/// - the last page at [`TOP_RA`];
/// - the page at 0 with doubleword 1 `low`, unless it is `None`.
fn memory(code: &[u32], low: Option<u64>) -> GuestMemory {
    let mut mem = GuestMemory::new(0x8_0000);
    map(&mut mem, CODE_VSID, 0, CODE_RA | RW);
    map(&mut mem, CODE_VSID, 0xFFFF, TOP_RA | RW);
    if let Some(d1) = low {
        map(&mut mem, LOW_VSID, 0, d1);
    }
    for (i, word) in code.iter().enumerate() {
        put(&mut mem, CODE_RA + 4 * i as u64, &word.to_be_bytes());
    }
    mem
}

/// A unit at the code page with MSR `msr`, the page table at [`HTAB`],
/// and SLB entries over the code segment, the last segment and segment 0.
fn kernel_unit(msr: u64) -> PpuExecutionUnit {
    let mut sup = SupervisorState::new(msr);
    sup.slb[0] = SlbEntry {
        esid_v: CODE_EA | SLB_VALID,
        vsid_flags: CODE_VSID << 12,
    };
    sup.slb[1] = SlbEntry {
        esid_v: (TOP_PAGE & !0x0FFF_FFFF) | SLB_VALID,
        vsid_flags: CODE_VSID << 12,
    };
    sup.slb[2] = SlbEntry {
        esid_v: SLB_VALID,
        vsid_flags: LOW_VSID << 12,
    };
    sup.partition = Partition::new(HTAB, 1 << 62, 0, Partition::INITIAL.lb()).unwrap();
    let mut unit = PpuExecutionUnit::new(UnitId::new(0));
    unit.state_mut().set_supervisor(Some(sup));
    unit.state_mut().pc = CODE_EA;
    unit
}

fn run(
    unit: &mut PpuExecutionUnit,
    mem: &GuestMemory,
    budget: u64,
) -> (ExecutionStepResult, Vec<Effect>) {
    let mut effects = Vec::new();
    let r = unit.run_until_yield(
        Budget::new(budget),
        &ExecutionContext::new(mem),
        &mut effects,
    );
    (r, effects)
}

/// Every data read intent's start and length: the reads outside the code
/// page and the page table.
fn reads(effects: &[Effect]) -> Vec<(u64, u64)> {
    effects
        .iter()
        .filter_map(|e| match e {
            Effect::SharedReadIntent { range, .. } => Some((range.start().raw(), range.length())),
            _ => None,
        })
        .filter(|&(at, _)| !(CODE_RA..CODE_RA + 0x1000).contains(&at) && at < HTAB)
        .collect()
}

/// Every write intent's start and bytes, in order.
fn writes(effects: &[Effect]) -> Vec<(u64, Vec<u8>)> {
    effects
        .iter()
        .filter_map(|e| match e {
            Effect::SharedWriteIntent { range, bytes, .. } => {
                Some((range.start().raw(), bytes.bytes().to_vec()))
            }
            _ => None,
        })
        .collect()
}

fn assert_ran(r: &ExecutionStepResult) {
    assert_eq!(
        r.yield_reason,
        YieldReason::BudgetExhausted,
        "{:?}",
        r.local_diagnostics
    );
}

fn lbz(rt: u32, d: u16, ra: u32) -> u32 {
    (34 << 26) | (rt << 21) | (ra << 16) | u32::from(d)
}

fn lwz(rt: u32, d: u16, ra: u32) -> u32 {
    (32 << 26) | (rt << 21) | (ra << 16) | u32::from(d)
}

fn lmw(rt: u32, d: u16, ra: u32) -> u32 {
    (46 << 26) | (rt << 21) | (ra << 16) | u32::from(d)
}

fn stb(rs: u32, d: u16, ra: u32) -> u32 {
    (38 << 26) | (rs << 21) | (ra << 16) | u32::from(d)
}

fn sth(rs: u32, d: u16, ra: u32) -> u32 {
    (44 << 26) | (rs << 21) | (ra << 16) | u32::from(d)
}

fn stw(rs: u32, d: u16, ra: u32) -> u32 {
    (36 << 26) | (rs << 21) | (ra << 16) | u32::from(d)
}

/// [CBE-Handbook p:253 s:9.5.8] A load or store that takes no alignment interrupt but crosses a virtual page boundary is performed through microcode.
#[test]
fn a_load_that_wraps_the_effective_space_reads_the_last_page_then_the_page_at_0() {
    let mut mem = memory(&[lwz(4, 0xFFE, 5)], Some(LOW_RA | RW));
    put(&mut mem, TOP_RA + 0xFFE, &[0x11, 0x22]);
    put(&mut mem, LOW_RA, &[0x33, 0x44]);
    let mut unit = kernel_unit(MSR_SF | MSR_IR | MSR_DR);
    unit.state_mut().set_gpr(5, TOP_PAGE);
    let (r, effects) = run(&mut unit, &mem, 1);
    assert_ran(&r);
    assert_eq!(unit.state().gpr[4], 0x1122_3344);
    assert_eq!(reads(&effects), vec![(TOP_RA + 0xFFE, 2), (LOW_RA, 2)]);
}

#[test]
fn a_store_that_wraps_the_effective_space_writes_the_last_page_then_the_page_at_0() {
    let mem = memory(&[stw(4, 0xFFE, 5)], Some(LOW_RA | RW));
    let mut unit = kernel_unit(MSR_SF | MSR_IR | MSR_DR);
    unit.state_mut().set_gpr(4, 0xAABB_CCDD);
    unit.state_mut().set_gpr(5, TOP_PAGE);
    let (r, effects) = run(&mut unit, &mem, 1);
    assert_ran(&r);
    assert_eq!(
        writes(&effects),
        vec![
            (TOP_RA + 0xFFE, vec![0xAA, 0xBB]),
            (LOW_RA, vec![0xCC, 0xDD])
        ]
    );
}

#[test]
fn loads_after_a_wrapping_store_in_one_batch_read_the_stored_bytes() {
    // The word wraps. The byte below 2^64 and the byte at 0 each lie in
    // one half of the word.
    let code = [
        stw(4, 0xFFE, 5),
        lwz(6, 0xFFE, 5),
        lbz(7, 0xFFF, 5),
        lbz(8, 0, 0),
    ];
    let mem = memory(&code, Some(LOW_RA | RW));
    let mut unit = kernel_unit(MSR_SF | MSR_IR | MSR_DR);
    unit.state_mut().set_gpr(4, 0xAABB_CCDD);
    unit.state_mut().set_gpr(5, TOP_PAGE);
    let (r, _) = run(&mut unit, &mem, 4);
    assert_ran(&r);
    let gpr = &unit.state().gpr;
    assert_eq!((gpr[6], gpr[7], gpr[8]), (0xAABB_CCDD, 0xBB, 0xCC));
}

#[test]
fn a_store_to_the_last_byte_of_the_effective_space_writes_its_real_byte() {
    let mem = memory(&[stb(4, 0xFFF, 5)], None);
    let mut unit = kernel_unit(MSR_SF | MSR_IR | MSR_DR);
    unit.state_mut().set_gpr(4, 0x5A);
    unit.state_mut().set_gpr(5, TOP_PAGE);
    let (r, effects) = run(&mut unit, &mem, 1);
    assert_ran(&r);
    assert_eq!(writes(&effects), vec![(TOP_RA + 0xFFF, vec![0x5A])]);
}

#[test]
fn an_lmw_from_the_last_word_of_the_effective_space_loads_its_next_word_from_0() {
    let mut mem = memory(&[lmw(30, 0xFFC, 5)], Some(LOW_RA | RW));
    put(&mut mem, TOP_RA + 0xFFC, &0x0102_0304u32.to_be_bytes());
    put(&mut mem, LOW_RA, &0x0506_0708u32.to_be_bytes());
    let mut unit = kernel_unit(MSR_SF | MSR_IR | MSR_DR);
    unit.state_mut().set_gpr(5, TOP_PAGE);
    let (r, effects) = run(&mut unit, &mem, 1);
    assert_ran(&r);
    assert_eq!(
        (unit.state().gpr[30], unit.state().gpr[31]),
        (0x0102_0304, 0x0506_0708)
    );
    assert_eq!(reads(&effects), vec![(TOP_RA + 0xFFC, 4), (LOW_RA, 4)]);
}

/// The guest fault code of a run's terminal stop, and the vector of the
/// interrupt it recorded.
fn stop(r: &ExecutionStepResult) -> (Option<u32>, Option<u64>) {
    let code = match r.fault {
        Some(cellgov_effects::FaultKind::Guest(code)) => Some(code),
        _ => None,
    };
    (code, r.local_diagnostics.interrupt.map(|i| i.vector))
}

/// The unaligned operands that run from the last page to the page at 0,
/// each with the effective address its instruction computes.
fn wrapping_unaligned_operands() -> [(u32, u64); 2] {
    [
        (lwz(4, 0xFFE, 5), TOP_PAGE + 0xFFE),
        (sth(4, 0xFFF, 5), TOP_PAGE + 0xFFF),
    ]
}

/// The SRR0 and DAR the run's interrupt recorded.
fn srr0_and_dar(r: &ExecutionStepResult) -> Option<(u64, Option<u64>)> {
    r.local_diagnostics.interrupt.map(|i| (i.srr0, i.dar))
}

/// [CBE-Handbook p:253 s:9.5.8] The PPE takes the alignment interrupt for a load or store whose operand is unaligned and in caching-inhibited storage.
/// [PPC-Book3 p:70 s:5.5.8] SRR0 is set to the instruction that caused the interrupt, and DAR to the effective address the instruction computed.
#[test]
fn an_unaligned_access_that_wraps_into_a_caching_inhibited_page_takes_the_alignment_interrupt() {
    for (insn, ea) in wrapping_unaligned_operands() {
        let mem = memory(&[insn], Some(LOW_RA | RW | PTE_I));
        let mut unit = kernel_unit(MSR_SF | MSR_IR | MSR_DR);
        unit.state_mut().set_gpr(5, TOP_PAGE);
        let (r, effects) = run(&mut unit, &mem, 1);
        assert_eq!(
            stop(&r),
            (None, Some(irq::VECTOR_ALIGNMENT)),
            "insn {insn:#x}"
        );
        assert_eq!(
            srr0_and_dar(&r),
            Some((CODE_EA, Some(ea))),
            "insn {insn:#x}"
        );
        assert!(writes(&effects).is_empty(), "insn {insn:#x}");
    }
}

/// [CBE-Handbook p:253 s:9.5.8] An unaligned operand in caching-inhibited storage takes the alignment interrupt, so the access reaches no storage.
/// [PPC-Book3 p:70 s:5.5.8] SRR0 is set to the instruction that caused the interrupt, and DAR to the effective address the instruction computed.
#[test]
fn an_unaligned_access_from_an_unbacked_last_page_into_an_inhibited_page_at_0_takes_alignment() {
    for (insn, ea) in wrapping_unaligned_operands() {
        let mut mem = memory(&[insn], Some(LOW_RA | RW | PTE_I));
        map(&mut mem, CODE_VSID, 0xFFFF, UNBACKED_RA | RW);
        let mut unit = kernel_unit(MSR_SF | MSR_IR | MSR_DR | MSR_ME);
        unit.state_mut().set_gpr(5, TOP_PAGE);
        let (r, effects) = run(&mut unit, &mem, 1);
        assert_eq!(
            stop(&r),
            (None, Some(irq::VECTOR_ALIGNMENT)),
            "insn {insn:#x}"
        );
        assert_eq!(
            srr0_and_dar(&r),
            Some((CODE_EA, Some(ea))),
            "insn {insn:#x}"
        );
        assert!(writes(&effects).is_empty(), "insn {insn:#x}");
    }
}
