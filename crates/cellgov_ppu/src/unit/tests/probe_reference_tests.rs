//! An access that runs from a caching-inhibited page with no storage
//! into the next page. The table lookup for the next page writes R to the
//! entry it matches, except when a store takes the alignment interrupt.
//! An access that runs from a cacheable page with no storage checkstops
//! first.

use crate::fault_codes::FAULT_CHECKSTOP;
use crate::mmu::Partition;
use crate::supervisor::{SlbEntry, SupervisorState};
use crate::*;
use cellgov_effects::Effect;
use cellgov_event::UnitId;
use cellgov_exec::{ExecutionContext, ExecutionStepResult, ExecutionUnit, YieldReason};
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_ps3_abi::hw::ppu_interrupt as irq;
use cellgov_ps3_abi::hw::ppu_mmu::{PTE_C, PTE_C_BYTE, PTE_I, PTE_R, PTE_R_BYTE};
use cellgov_ps3_abi::hw::ppu_supervisor::{MSR_DR, MSR_IR, MSR_ME, MSR_SF, SLB_VALID};
use cellgov_time::Budget;

/// The page table: 2^11 groups from 256 KB to the end of the memory.
const HTAB: u64 = 0x4_0000;
/// The segment that the SLB maps, and its VSID.
const SEGMENT: u64 = 0xC000_0000;
const VSID: u64 = 0x123;
/// The real address of page 0, which holds the code.
const CODE_RA: u64 = 0x3000;
/// Page 4, where each operand starts, is caching-inhibited at a real
/// address past the 512 KB the memory holds.
const START_EA: u64 = SEGMENT + 0x4000;
const UNBACKED_RA: u64 = 0x9_0000;
/// Page 5, where each operand ends, and its real address.
const NEXT: u64 = 5;
const NEXT_RA: u64 = 0x7000;
/// Bytes in a group of page table entries.
const GROUP: u64 = 128;

fn lwz(rt: u32, d: u16, ra: u32) -> u32 {
    (32 << 26) | (rt << 21) | (ra << 16) | u32::from(d)
}

fn sth(rs: u32, d: u16, ra: u32) -> u32 {
    (44 << 26) | (rs << 21) | (ra << 16) | u32::from(d)
}

fn stb(rs: u32, d: u16, ra: u32) -> u32 {
    (38 << 26) | (rs << 21) | (ra << 16) | u32::from(d)
}

fn put(mem: &mut GuestMemory, at: u64, bytes: &[u8]) {
    let range = ByteRange::new(GuestAddr::new(at), bytes.len() as u64).unwrap();
    mem.apply_commit(range, bytes).unwrap();
}

/// The real address of the entry for page `index` of the segment: slot 0
/// of its primary group, whose hash is VSID XOR the page index.
fn entry(index: u64) -> u64 {
    HTAB + ((VSID ^ index) & 0x7FF) * GROUP
}

/// Writes the entry for page `index` with doubleword 1 `d1`.
fn map_page(mem: &mut GuestMemory, index: u64, d1: u64) {
    let d0 = (VSID << 12) | 1;
    put(mem, entry(index), &d0.to_be_bytes());
    put(mem, entry(index) + 8, &d1.to_be_bytes());
}

/// 512 KB of memory that holds:
///
/// - the page table;
/// - `code` on page 0, whose entry has R and C set;
/// - the entry of page 4, read/write and caching-inhibited at
///   [`UNBACKED_RA`], with R clear;
/// - the entry of page 5 at [`NEXT_RA`] with doubleword 1 low bits
///   `next`, when `next` is `Some`.
fn memory(code: &[u32], next: Option<u64>) -> GuestMemory {
    let mut mem = GuestMemory::new(0x8_0000);
    map_page(&mut mem, 0, CODE_RA | PTE_R | PTE_C | 0b10);
    map_page(&mut mem, 4, UNBACKED_RA | PTE_I | 0b10);
    if let Some(bits) = next {
        map_page(&mut mem, NEXT, NEXT_RA | bits);
    }
    for (i, word) in code.iter().enumerate() {
        put(&mut mem, CODE_RA + 4 * i as u64, &word.to_be_bytes());
    }
    mem
}

/// A unit at the segment's first word, with:
///
/// - relocation on and MSR\[ME\] = 1;
/// - SLB entry 0 over the segment;
/// - the page table at [`HTAB`];
/// - r5 at [`START_EA`].
fn kernel_unit() -> PpuExecutionUnit {
    let mut sup = SupervisorState::new(MSR_SF | MSR_IR | MSR_DR | MSR_ME);
    sup.slb[0] = SlbEntry {
        esid_v: SEGMENT | SLB_VALID,
        vsid_flags: VSID << 12,
    };
    sup.partition = Partition::new(HTAB, 1 << 62, 0, Partition::INITIAL.lb()).unwrap();
    let mut unit = PpuExecutionUnit::new(UnitId::new(0));
    unit.state_mut().set_supervisor(Some(sup));
    unit.state_mut().pc = SEGMENT;
    unit.state_mut().set_gpr(5, START_EA);
    unit
}

/// One batch of at most `budget` instructions.
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

/// The bytes of the one-byte write intents to byte `byte` of the entry
/// of page `index`, in order.
fn entry_byte_writes(effects: &[Effect], index: u64, byte: usize) -> Vec<u8> {
    let at = entry(index) + byte as u64;
    effects
        .iter()
        .filter_map(|e| match e {
            Effect::SharedWriteIntent { range, bytes, .. } if range.start().raw() == at => {
                Some(bytes.bytes().to_vec())
            }
            _ => None,
        })
        .flatten()
        .collect()
}

/// The read intents in the page table, in order, as start and length.
fn table_reads(effects: &[Effect]) -> Vec<(u64, u64)> {
    let table = HTAB..HTAB + 0x4_0000;
    effects
        .iter()
        .filter_map(|e| match e {
            Effect::SharedReadIntent { range, .. } if table.contains(&range.start().raw()) => {
                Some((range.start().raw(), range.length()))
            }
            _ => None,
        })
        .collect()
}

/// The vector and DSISR of the interrupt the run took.
fn interrupt(r: &ExecutionStepResult) -> Option<(u64, Option<u32>)> {
    r.local_diagnostics.interrupt.map(|i| (i.vector, i.dsisr))
}

/// Byte 14 of the entry for page 5 with low bits `bits`, after the
/// table lookup writes R.
///
/// [PPC-Book3 p:2 s:1.3.2] When the processor writes a table field, no other defined field changes.
fn next_r_byte(bits: u64) -> u8 {
    (NEXT_RA | bits | PTE_R).to_be_bytes()[PTE_R_BYTE - 8]
}

/// [CBE-Handbook p:90 s:4.2.6.8] The PPE sets R regardless of any storage interrupt, and does not set C for an access that takes a storage exception.
/// [CBE-Handbook p:290 s:9.10] A store's page-protection interrupt precedes the alignment interrupt of caching-inhibited storage when C is 0.
#[test]
fn a_store_the_next_page_refuses_writes_that_entrys_r_and_publishes_the_group_it_read() {
    // Page 5 is read-only: caching-inhibited with C = 0, or cacheable.
    // The sth starts in the last byte of page 4.
    let cases = [0b11 | PTE_I, 0b11];
    for bits in cases {
        let mem = memory(&[sth(4, 0xFFF, 5)], Some(bits));
        let mut unit = kernel_unit();
        let (r, effects) = run(&mut unit, &mem, 1);
        assert_eq!(
            interrupt(&r),
            Some((
                irq::VECTOR_DATA_STORAGE,
                Some(irq::DSISR_PROTECTION | irq::DSISR_STORE)
            )),
            "page 5 {bits:#x}"
        );
        assert_eq!(
            entry_byte_writes(&effects, NEXT, PTE_R_BYTE),
            vec![next_r_byte(bits)],
            "page 5 {bits:#x}"
        );
        assert_eq!(
            entry_byte_writes(&effects, NEXT, PTE_C_BYTE),
            Vec::<u8>::new(),
            "page 5 {bits:#x}"
        );
        assert!(
            table_reads(&effects).contains(&(entry(NEXT), GROUP)),
            "page 5 {bits:#x}: {:x?}",
            table_reads(&effects)
        );
    }
}

/// [CBE-Handbook p:253 s:9.5.8] A cacheable load or store that crosses a virtual page boundary takes no alignment interrupt; the PPE sends it to microcode.
/// [PPC-Book3 p:66 s:5.5.2] An access to nonexistent storage may enter the checkstop state.
#[test]
fn an_unaligned_access_out_of_an_unbacked_cacheable_page_checkstops_where_the_next_page_refuses() {
    // Page 4 is cacheable here. Page 5 refuses each operand: read-only
    // for the sth, unmapped for the lwz.
    let cases = [(sth(4, 0xFFF, 5), Some(0b11)), (lwz(4, 0xFFE, 5), None)];
    for (insn, next) in cases {
        let mut mem = memory(&[insn], next);
        map_page(&mut mem, 4, UNBACKED_RA | 0b10);
        let mut unit = kernel_unit();
        let (r, _) = run(&mut unit, &mem, 1);
        assert_eq!(
            r.fault,
            Some(cellgov_effects::FaultKind::Guest(FAULT_CHECKSTOP)),
            "insn {insn:#x}"
        );
        assert_eq!(interrupt(&r), None, "insn {insn:#x}");
    }
}

/// [CBE-Handbook p:90 s:4.2.6.8] A load's table lookup is nonspeculative, so a lookup that matches an entry with R = 0 sets R.
/// [CBE-Handbook p:253 s:9.5.8] The alignment interrupt is taken when the data access cannot be performed in caching-inhibited storage.
#[test]
fn an_unaligned_load_into_a_next_page_that_translates_writes_that_entrys_r_once() {
    let bits = 0b10;
    let mem = memory(&[lwz(4, 0xFFE, 5)], Some(bits));
    let mut unit = kernel_unit();
    let (r, effects) = run(&mut unit, &mem, 1);
    assert_eq!(
        interrupt(&r).map(|(vector, _)| vector),
        Some(irq::VECTOR_ALIGNMENT)
    );
    assert_eq!(
        entry_byte_writes(&effects, NEXT, PTE_R_BYTE),
        vec![next_r_byte(bits)]
    );
}

/// [CBE-Handbook p:90 s:4.2.6.8] Footnote 4: a misaligned caching-inhibited store takes the alignment interrupt before the table lookup, so the lookup writes no R or C.
/// [CBE-Handbook p:253 s:9.5.8] The alignment interrupt is taken when the data access cannot be performed in caching-inhibited storage.
#[test]
fn an_unaligned_store_into_a_next_page_that_translates_writes_no_r_or_c_and_reads_no_group() {
    // Each case is page 5's low bits. In the second case the store's
    // table lookup refuses it, and the store meets the page's load
    // translation.
    let cases = [0b10, 0b11 | PTE_I | PTE_C];
    for bits in cases {
        let mem = memory(&[sth(4, 0xFFF, 5)], Some(bits));
        let mut unit = kernel_unit();
        let (r, effects) = run(&mut unit, &mem, 1);
        assert_eq!(
            interrupt(&r).map(|(vector, _)| vector),
            Some(irq::VECTOR_ALIGNMENT),
            "page 5 {bits:#x}"
        );
        assert_eq!(
            entry_byte_writes(&effects, NEXT, PTE_R_BYTE),
            Vec::<u8>::new(),
            "page 5 {bits:#x}"
        );
        assert_eq!(
            entry_byte_writes(&effects, NEXT, PTE_C_BYTE),
            Vec::<u8>::new(),
            "page 5 {bits:#x}"
        );
        assert!(
            !table_reads(&effects).contains(&(entry(NEXT), GROUP)),
            "page 5 {bits:#x}: {:x?}",
            table_reads(&effects)
        );
    }
}

/// [CBE-Handbook p:90 s:4.2.6.8] The PPE performs every store, its page table entry updates included, in order.
#[test]
fn a_next_page_search_whose_r_byte_a_buffered_store_writes_waits_for_that_store() {
    // Page 3 maps the page of real memory that holds the entries of pages
    // 0, 3, 4 and 5. Its own entry has R and C set, so a store through it
    // needs no entry write. The stb writes byte 14 of page 5's entry with
    // R = 0 and reserved bit 53 set.
    let page_of_entries = entry(NEXT) & !0xFFF;
    let mut mem = memory(&[stb(6, 0, 7), lwz(4, 0xFFE, 5)], Some(0b10));
    map_page(&mut mem, 3, page_of_entries | PTE_R | PTE_C | 0b10);
    let mut unit = kernel_unit();
    unit.state_mut().set_gpr(6, 0x74);
    unit.state_mut()
        .set_gpr(7, SEGMENT + 0x3000 + (entry(NEXT) + 14 - page_of_entries));
    let (r, effects) = run(&mut unit, &mem, 2);
    assert_eq!(r.yield_reason, YieldReason::BudgetExhausted);
    assert_eq!(unit.retired_instructions(), 1, "the lwz waits for the stb");
    assert_eq!(
        entry_byte_writes(&effects, NEXT, PTE_R_BYTE),
        vec![0x74],
        "only the stb"
    );
    for e in &effects {
        if let Effect::SharedWriteIntent { range, bytes, .. } = e {
            put(&mut mem, range.start().raw(), bytes.bytes());
        }
    }
    let (r, effects) = run(&mut unit, &mem, 1);
    assert_eq!(
        interrupt(&r).map(|(vector, _)| vector),
        Some(irq::VECTOR_ALIGNMENT)
    );
    assert_eq!(
        entry_byte_writes(&effects, NEXT, PTE_R_BYTE),
        vec![0x75],
        "R set over the stored byte"
    );
}
