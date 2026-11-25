//! A misaligned store that takes the alignment interrupt in
//! caching-inhibited storage keeps no R or C write and no group read of
//! its own table searches. This is true where storage holds its pages and
//! where no storage holds them. These searches keep their writes:
//!
//! - the search of a load;
//! - the search of the fetch;
//! - the search of an earlier instruction;
//! - the search of a store that caching-inhibited storage refuses at any
//!   alignment.

use crate::mmu::Partition;
use crate::supervisor::{SlbEntry, SupervisorState};
use crate::*;
use cellgov_effects::Effect;
use cellgov_event::UnitId;
use cellgov_exec::{ExecutionContext, ExecutionStepResult, ExecutionUnit};
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
/// Doubleword 1 of page 0's entry: read/write with R = C = 0.
const CODE_D1: u64 = CODE_RA | 0b10;
/// Page 4, where each access starts, and its real address.
const PAGE: u64 = 4;
const PAGE_EA: u64 = SEGMENT + 0x4000;
const PAGE_RA: u64 = 0x6000;
/// A real address past the 512 KB the memory holds.
const UNBACKED_RA: u64 = 0x9_0000;
/// Page 5, after page 4, and its real address.
const NEXT: u64 = 5;
const NEXT_RA: u64 = 0x7000;
/// Bytes in a group of page table entries.
const GROUP: u64 = 128;

fn lwz(rt: u32, d: u16, ra: u32) -> u32 {
    (32 << 26) | (rt << 21) | (ra << 16) | u32::from(d)
}

fn stw(rs: u32, d: u16, ra: u32) -> u32 {
    (36 << 26) | (rs << 21) | (ra << 16) | u32::from(d)
}

fn sth(rs: u32, d: u16, ra: u32) -> u32 {
    (44 << 26) | (rs << 21) | (ra << 16) | u32::from(d)
}

/// `dcbz ra,rb`.
fn dcbz(ra: u32, rb: u32) -> u32 {
    (31 << 26) | (ra << 16) | (rb << 11) | (1014 << 1)
}

/// `stwcx. rs,0,rb`.
fn stwcx(rs: u32, rb: u32) -> u32 {
    (31 << 26) | (rs << 21) | (rb << 11) | (150 << 1) | 1
}

/// `stdcx. rs,0,rb`.
fn stdcx(rs: u32, rb: u32) -> u32 {
    (31 << 26) | (rs << 21) | (rb << 11) | (214 << 1) | 1
}

/// `stmw rs,d(ra)`.
fn stmw(rs: u32, d: u16, ra: u32) -> u32 {
    (47 << 26) | (rs << 21) | (ra << 16) | u32::from(d)
}

/// `stswi rs,ra,nb`.
fn stswi(rs: u32, ra: u32, nb: u32) -> u32 {
    (31 << 26) | (rs << 21) | (ra << 16) | (nb << 11) | (725 << 1)
}

/// `stswx rs,0,rb`.
fn stswx(rs: u32, rb: u32) -> u32 {
    (31 << 26) | (rs << 21) | (rb << 11) | (661 << 1)
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
/// - `code` on page 0, whose entry has doubleword 1 [`CODE_D1`];
/// - the entry of page 4 with doubleword 1 `page`;
/// - the entry of page 5 with doubleword 1 `next`, when `next` is `Some`.
fn memory(code: &[u32], page: u64, next: Option<u64>) -> GuestMemory {
    let mut mem = GuestMemory::new(0x8_0000);
    map_page(&mut mem, 0, CODE_D1);
    map_page(&mut mem, PAGE, page);
    if let Some(d1) = next {
        map_page(&mut mem, NEXT, d1);
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
/// - r5 at [`PAGE_EA`].
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
    unit.state_mut().set_gpr(5, PAGE_EA);
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

/// The vector of the interrupt the run took.
fn vector(r: &ExecutionStepResult) -> Option<u64> {
    r.local_diagnostics.interrupt.map(|i| i.vector)
}

/// Byte 14 of an entry with doubleword 1 `d1`, after a search writes R.
///
/// [PPC-Book3 p:2 s:1.3.2] When the processor writes a table field, no other defined field changes.
fn r_byte(d1: u64) -> u8 {
    (d1 | PTE_R).to_be_bytes()[PTE_R_BYTE - 8]
}

/// Byte 15 of an entry with doubleword 1 `d1`, after a search writes C.
///
/// [PPC-Book3 p:2 s:1.3.2] When the processor writes a table field, no other defined field changes.
fn c_byte(d1: u64) -> u8 {
    (d1 | PTE_C).to_be_bytes()[PTE_C_BYTE - 8]
}

/// Asserts that `effects` hold no R or C write to the entry of page
/// `index` and no read of its group.
fn assert_no_search_of(effects: &[Effect], index: u64, case: &str) {
    assert_eq!(
        entry_byte_writes(effects, index, PTE_R_BYTE),
        Vec::<u8>::new(),
        "{case}: page {index} R"
    );
    assert_eq!(
        entry_byte_writes(effects, index, PTE_C_BYTE),
        Vec::<u8>::new(),
        "{case}: page {index} C"
    );
    assert!(
        !table_reads(effects).contains(&(entry(index), GROUP)),
        "{case}: page {index} group: {:x?}",
        table_reads(effects)
    );
}

/// [CBE-Handbook p:90 s:4.2.6.8] Footnote 4: a misaligned caching-inhibited store is speculative, so its table lookup is not performed when it takes the alignment interrupt.
/// [PPC-Book3 p:44 s:4.9] Figure 29: an in-order store that is not performed may set R and C, so the PPE handbook decides.
/// [CBE-Handbook p:290 s:9.10] A store's alignment interrupt in caching-inhibited storage precedes the page-protection data storage interrupt when C is 1.
#[test]
fn a_misaligned_inhibited_store_in_a_page_storage_holds_writes_no_r_or_c_and_reads_no_group() {
    // (instruction, page 4's low bits).
    let cases = [
        (sth(4, 1, 5), 0b10 | PTE_I),
        (sth(4, 1, 5), 0b10 | PTE_I | PTE_R),
        (sth(4, 1, 5), 0b11 | PTE_I | PTE_C),
    ];
    for (insn, bits) in cases {
        let mem = memory(&[insn], PAGE_RA | bits, None);
        let mut unit = kernel_unit();
        let (r, effects) = run(&mut unit, &mem, 1);
        let case = format!("insn {insn:#x} page 4 {bits:#x}");
        assert_eq!(vector(&r), Some(irq::VECTOR_ALIGNMENT), "{case}");
        assert_no_search_of(&effects, PAGE, &case);
    }
}

/// [CBE-Handbook p:90 s:4.2.6.8] Footnote 4: a misaligned caching-inhibited store is speculative, so its table lookup is not performed when it takes the alignment interrupt.
/// [CBE-Handbook p:253 s:9.5.8] The alignment interrupt is taken when the data access cannot be performed in caching-inhibited storage.
#[test]
fn a_misaligned_inhibited_store_across_two_pages_storage_holds_writes_no_r_or_c_to_either() {
    let mem = memory(
        &[sth(4, 0xFFF, 5)],
        PAGE_RA | PTE_I | 0b10,
        Some(NEXT_RA | 0b10),
    );
    let mut unit = kernel_unit();
    let (r, effects) = run(&mut unit, &mem, 1);
    assert_eq!(vector(&r), Some(irq::VECTOR_ALIGNMENT));
    assert_no_search_of(&effects, PAGE, "page 4");
    assert_no_search_of(&effects, NEXT, "page 5");
}

/// [CBE-Handbook p:90 s:4.2.6.8] Footnote 4: a misaligned caching-inhibited store is speculative, so its table lookup is not performed when it takes the alignment interrupt.
/// [CBE-Handbook p:253 s:9.5.8] The alignment interrupt is taken when the data access cannot be performed in caching-inhibited storage.
#[test]
fn a_misaligned_inhibited_store_out_of_a_page_no_storage_holds_writes_no_r_or_c_and_reads_no_group()
{
    // (instruction, page 5's doubleword 1). The second sth crosses into
    // page 5, where storage holds the page.
    let cases = [
        (sth(4, 1, 5), None),
        (sth(4, 0xFFF, 5), Some(NEXT_RA | 0b10)),
    ];
    for (insn, next) in cases {
        let mem = memory(&[insn], UNBACKED_RA | PTE_I | 0b10, next);
        let mut unit = kernel_unit();
        let (r, effects) = run(&mut unit, &mem, 1);
        let case = format!("insn {insn:#x}");
        assert_eq!(vector(&r), Some(irq::VECTOR_ALIGNMENT), "{case}");
        assert_no_search_of(&effects, PAGE, &case);
        assert_no_search_of(&effects, NEXT, &case);
    }
}

/// [CBE-Handbook p:90 s:4.2.6.8] A load's table lookup is nonspeculative, so a lookup that matches an entry with R = 0 sets R.
/// [CBE-Handbook p:253 s:9.5.8] The alignment interrupt is taken when the data access cannot be performed in caching-inhibited storage.
#[test]
fn a_misaligned_inhibited_load_keeps_the_r_write_and_the_group_read_of_its_search() {
    let page = PAGE_RA | PTE_I | 0b10;
    let mem = memory(&[lwz(4, 1, 5)], page, None);
    let mut unit = kernel_unit();
    let (r, effects) = run(&mut unit, &mem, 1);
    assert_eq!(vector(&r), Some(irq::VECTOR_ALIGNMENT));
    assert_eq!(
        entry_byte_writes(&effects, PAGE, PTE_R_BYTE),
        vec![r_byte(page)]
    );
    assert!(
        table_reads(&effects).contains(&(entry(PAGE), GROUP)),
        "{:x?}",
        table_reads(&effects)
    );
}

/// [CBE-Handbook p:90 s:4.2.6.8] Each table lookup but that of a misaligned caching-inhibited store is nonspeculative, so it sets R whatever interrupt the access takes.
/// [CBE-Handbook p:90 s:4.2.6.8] A store's lookup sets C = 0 to 1 unless the access takes a storage exception.
/// [CBE-Handbook p:253 s:9.5.8] A dcbz in caching-inhibited storage takes the alignment interrupt.
/// [CBE-Handbook p:253 s:9.5.8] An stmw, stswi, stswx, stwcx. or stdcx. operand in caching-inhibited storage takes the alignment interrupt, aligned or not.
#[test]
fn an_aligned_store_that_takes_alignment_in_inhibited_storage_keeps_its_r_and_c_writes() {
    // Each operand starts at page 4's first byte, so none is misaligned.
    // XER gives the stswx a count of 4 bytes.
    let page = PAGE_RA | PTE_I | 0b10;
    let insns = [
        dcbz(0, 5),
        stwcx(4, 5),
        stdcx(4, 5),
        stmw(30, 0, 5),
        stswi(4, 5, 4),
        stswx(4, 5),
    ];
    for insn in insns {
        let mem = memory(&[insn], page, None);
        let mut unit = kernel_unit();
        unit.state_mut().set_xer(4);
        let (r, effects) = run(&mut unit, &mem, 1);
        let case = format!("insn {insn:#x}");
        assert_eq!(vector(&r), Some(irq::VECTOR_ALIGNMENT), "{case}");
        assert_eq!(
            entry_byte_writes(&effects, PAGE, PTE_R_BYTE),
            vec![r_byte(page)],
            "{case}"
        );
        assert_eq!(
            entry_byte_writes(&effects, PAGE, PTE_C_BYTE),
            vec![c_byte(page)],
            "{case}"
        );
        assert!(
            table_reads(&effects).contains(&(entry(PAGE), GROUP)),
            "{case}: {:x?}",
            table_reads(&effects)
        );
    }
}

/// `stvlx vs,0,rb`, or `stvlxl` with `last`.
fn stvlx(vs: u32, rb: u32, last: bool) -> u32 {
    let xo = if last { 903 } else { 647 };
    (31 << 26) | (vs << 21) | (rb << 11) | (xo << 1)
}

/// [CBE-Handbook p:90 s:4.2.6.8] Each table lookup but that of a misaligned caching-inhibited store is nonspeculative, so it sets R whatever interrupt the access takes.
/// [CBE-Handbook p:90 s:4.2.6.8] A store's lookup sets C = 0 to 1 unless the access takes a storage exception.
/// [CBE-Handbook p:290 s:9.10] Table 9-39: a VSU store left or right that references caching-inhibited storage takes the alignment interrupt.
#[test]
fn an_aligned_stvlx_that_takes_alignment_in_inhibited_storage_keeps_its_r_and_c_writes() {
    // The operand starts at page 4's first byte, so it is quadword
    // aligned and its 16 bytes stay in page 4.
    let page = PAGE_RA | PTE_I | 0b10;
    for last in [false, true] {
        let insn = stvlx(4, 5, last);
        let mem = memory(&[insn], page, None);
        let mut unit = kernel_unit();
        let mut sup = unit.state().supervisor().cloned().expect("supervisor");
        sup.msr |= cellgov_ps3_abi::hw::ppu_supervisor::MSR_VXU;
        unit.state_mut().set_supervisor(Some(sup));
        let (r, effects) = run(&mut unit, &mem, 1);
        let case = format!("insn {insn:#x}");
        assert_eq!(vector(&r), Some(irq::VECTOR_ALIGNMENT), "{case}");
        assert_eq!(
            entry_byte_writes(&effects, PAGE, PTE_R_BYTE),
            vec![r_byte(page)],
            "{case}"
        );
        assert_eq!(
            entry_byte_writes(&effects, PAGE, PTE_C_BYTE),
            vec![c_byte(page)],
            "{case}"
        );
    }
}

/// [CBE-Handbook p:90 s:4.2.6.8] A table lookup that matches an entry with R = 0 sets R, and a store's lookup sets C = 0 to 1.
/// [CBE-Handbook p:90 s:4.2.6.8] Footnote 4: a misaligned caching-inhibited store is speculative, so its table lookup is not performed when it takes the alignment interrupt.
#[test]
fn the_fetch_and_an_earlier_store_keep_their_writes_when_a_misaligned_inhibited_store_faults() {
    // The stw is aligned, so it retires and writes page 4's R and C. The
    // sth then faults, and the batch ends ahead of it.
    let page = PAGE_RA | PTE_I | 0b10;
    let mut mem = memory(&[stw(4, 0, 5), sth(4, 1, 5)], page, None);
    let mut unit = kernel_unit();
    let (_, effects) = run(&mut unit, &mem, 2);
    assert_eq!(unit.retired_instructions(), 1, "the sth does not retire");
    assert_eq!(
        entry_byte_writes(&effects, 0, PTE_R_BYTE),
        vec![r_byte(CODE_D1)]
    );
    assert_eq!(
        entry_byte_writes(&effects, PAGE, PTE_R_BYTE),
        vec![r_byte(page)]
    );
    assert_eq!(
        entry_byte_writes(&effects, PAGE, PTE_C_BYTE),
        vec![c_byte(page)]
    );
    for e in &effects {
        if let Effect::SharedWriteIntent { range, bytes, .. } = e {
            put(&mut mem, range.start().raw(), bytes.bytes());
        }
    }
    let (r, _) = run(&mut unit, &mem, 1);
    assert_eq!(vector(&r), Some(irq::VECTOR_ALIGNMENT), "the sth");
}
