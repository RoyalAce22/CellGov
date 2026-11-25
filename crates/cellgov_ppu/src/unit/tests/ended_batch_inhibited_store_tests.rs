//! A misaligned store that crosses from a cacheable page into a
//! caching-inhibited page, and whose batch ends ahead of it, keeps no R
//! write of its table searches in that batch. It takes the alignment
//! interrupt in the next batch, so the R bytes of its pages stay as they
//! were.

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

#[path = "ended_batch_store_prediction_tests.rs"]
mod prediction;

/// The page table: 2^11 groups from 256 KB to the end of the memory.
const HTAB: u64 = 0x4_0000;
/// The segment that the SLB maps, and its VSID.
const SEGMENT: u64 = 0xC000_0000;
const VSID: u64 = 0x123;
/// The real address of page 0, which holds the code.
const CODE_RA: u64 = 0x3000;
/// Doubleword 1 of page 0's entry: read/write with R = C = 0.
const CODE_D1: u64 = CODE_RA | 0b10;
/// Page 3, which maps the page of real memory that holds the entries of
/// pages 0, 3, 4, 5 and 6.
const ENTRIES: u64 = 3;
/// Page 4, where the sth starts, and its real address.
const PAGE: u64 = 4;
const PAGE_RA: u64 = 0x6000;
/// A real address past the 512 KB the memory holds.
const UNBACKED_RA: u64 = 0x9_0000;
/// Page 5, after page 4, and its real address.
const NEXT: u64 = 5;
const NEXT_RA: u64 = 0x7000;
/// Doubleword 1 of page 5's entry: read/write and caching-inhibited with
/// R = C = 0.
const NEXT_D1: u64 = NEXT_RA | PTE_I | 0b10;
/// Page 6, a second effective page over page 5's real page.
const ALIAS: u64 = 6;
/// Bytes in a group of page table entries.
const GROUP: u64 = 128;

fn stb(rs: u32, d: u16, ra: u32) -> u32 {
    (38 << 26) | (rs << 21) | (ra << 16) | u32::from(d)
}

fn stw(rs: u32, d: u16, ra: u32) -> u32 {
    (36 << 26) | (rs << 21) | (ra << 16) | u32::from(d)
}

fn sth(rs: u32, d: u16, ra: u32) -> u32 {
    (44 << 26) | (rs << 21) | (ra << 16) | u32::from(d)
}

fn put(mem: &mut GuestMemory, at: u64, bytes: &[u8]) {
    let range = ByteRange::new(GuestAddr::new(at), bytes.len() as u64).unwrap();
    mem.apply_commit(range, bytes).unwrap();
}

/// The byte of `mem` at real address `at`.
fn byte_at(mem: &GuestMemory, at: u64) -> u8 {
    let range = ByteRange::new(GuestAddr::new(at), 1).unwrap();
    mem.read(range).unwrap()[0]
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

/// The effective address, through page 3, of byte `byte` of the entry of
/// page `index`.
fn entry_byte_ea(index: u64, byte: usize) -> u64 {
    let page_of_entries = entry(ENTRIES) & !0xFFF;
    SEGMENT + (ENTRIES << 12) + (entry(index) + byte as u64 - page_of_entries)
}

/// 512 KB of memory that holds:
///
/// - the page table;
/// - `code` on page 0, whose entry has doubleword 1 [`CODE_D1`];
/// - the entry of page 3, read/write with R and C set;
/// - the entry of page 4 with doubleword 1 `page`;
/// - the entry of page 5 with doubleword 1 [`NEXT_D1`].
fn memory(code: &[u32], page: u64) -> GuestMemory {
    let mut mem = GuestMemory::new(0x8_0000);
    map_page(&mut mem, 0, CODE_D1);
    let page_of_entries = entry(ENTRIES) & !0xFFF;
    map_page(&mut mem, ENTRIES, page_of_entries | PTE_R | PTE_C | 0b10);
    map_page(&mut mem, PAGE, page);
    map_page(&mut mem, NEXT, NEXT_D1);
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
/// - r5 at page 4's first byte.
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
    unit.state_mut().set_gpr(5, SEGMENT + (PAGE << 12));
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

/// Commits the write intents of `effects` to `mem`.
fn commit(mem: &mut GuestMemory, effects: &[Effect]) {
    for e in effects {
        if let Effect::SharedWriteIntent { range, bytes, .. } = e {
            put(mem, range.start().raw(), bytes.bytes());
        }
    }
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

/// The writes in `effects` to byte 14 of the entry of page `index` that
/// set R. A guest store to that byte with R clear is not one of them.
fn r_writes(effects: &[Effect], index: u64) -> Vec<u8> {
    let r = PTE_R.to_be_bytes()[PTE_R_BYTE - 8];
    entry_byte_writes(effects, index, PTE_R_BYTE)
        .into_iter()
        .filter(|byte| byte & r != 0)
        .collect()
}

/// Runs `unit` for the two batches of the earlier store and the sth at
/// the end of page 4, and commits each batch's writes to `mem`.
///
/// Asserts these facts:
/// - the earlier store retires in the first batch, and the batch ends
///   ahead of the sth;
/// - the sth takes the alignment interrupt in the second batch;
/// - neither batch sets R in the entry of page 4 or of page 5.
fn run_to_alignment(unit: &mut PpuExecutionUnit, mem: &mut GuestMemory) {
    let (_, first) = run(unit, mem, 2);
    assert_eq!(unit.retired_instructions(), 1, "the sth waits");
    commit(mem, &first);
    let (r, second) = run(unit, mem, 1);
    commit(mem, &second);
    assert_eq!(
        r.local_diagnostics.interrupt.map(|i| i.vector),
        Some(irq::VECTOR_ALIGNMENT),
        "batch 2"
    );
    for (batch, effects) in [(1, &first), (2, &second)] {
        for index in [PAGE, NEXT] {
            assert_eq!(
                r_writes(effects, index),
                Vec::<u8>::new(),
                "batch {batch}: page {index} R"
            );
        }
    }
}

/// Asserts that the R byte of the entry of page `index` in `mem` still
/// holds the R byte of doubleword 1 `d1`.
fn assert_r_byte_unchanged(mem: &GuestMemory, index: u64, d1: u64) {
    assert_eq!(
        byte_at(mem, entry(index) + PTE_R_BYTE as u64),
        d1.to_be_bytes()[PTE_R_BYTE - 8],
        "page {index} R byte"
    );
}

/// [CBE-Handbook p:90 s:4.2.6.8] Footnote 4: a misaligned caching-inhibited store is speculative, so its table lookup is not performed when it takes the alignment interrupt.
/// [CBE-Handbook p:253 s:9.5.8] The PPE takes the alignment interrupt for a load or store whose operand is unaligned and in caching-inhibited storage.
#[test]
fn a_misaligned_inhibited_store_whose_next_page_search_waits_for_a_store_keeps_no_first_page_r() {
    // The stb writes byte 14 of page 5's entry with its value unchanged.
    // The search of page 5 waits for the stb, so the batch ends ahead of
    // the sth.
    let page = PAGE_RA | PTE_C | 0b10;
    let mut mem = memory(&[stb(6, 0, 7), sth(4, 0xFFF, 5)], page);
    let mut unit = kernel_unit();
    unit.state_mut()
        .set_gpr(6, u64::from(NEXT_D1.to_be_bytes()[PTE_R_BYTE - 8]));
    unit.state_mut().set_gpr(7, entry_byte_ea(NEXT, PTE_R_BYTE));
    run_to_alignment(&mut unit, &mut mem);
    assert_r_byte_unchanged(&mem, PAGE, page);
}

/// [CBE-Handbook p:90 s:4.2.6.8] Footnote 4: a misaligned caching-inhibited store is speculative, so its table lookup is not performed when it takes the alignment interrupt.
/// [CBE-Handbook p:253 s:9.5.8] The alignment interrupt is taken when the data access cannot be performed in caching-inhibited storage.
#[test]
fn a_misaligned_inhibited_store_from_a_page_no_storage_holds_whose_batch_ends_keeps_no_first_page_r(
) {
    // Page 4 is cacheable over a real address past the memory. The stb
    // writes byte 14 of page 5's entry with its value unchanged. The
    // search of page 5 waits for the stb, so the batch ends ahead of the
    // sth.
    let page = UNBACKED_RA | PTE_C | 0b10;
    let mut mem = memory(&[stb(6, 0, 7), sth(4, 0xFFF, 5)], page);
    let mut unit = kernel_unit();
    unit.state_mut()
        .set_gpr(6, u64::from(NEXT_D1.to_be_bytes()[PTE_R_BYTE - 8]));
    unit.state_mut().set_gpr(7, entry_byte_ea(NEXT, PTE_R_BYTE));
    run_to_alignment(&mut unit, &mut mem);
    assert_r_byte_unchanged(&mem, PAGE, page);
}

/// [CBE-Handbook p:90 s:4.2.6.8] Footnote 4: a misaligned caching-inhibited store is speculative, so its table lookup is not performed when it takes the alignment interrupt.
/// [CBE-Handbook p:253 s:9.5.8] The PPE takes the alignment interrupt for a load or store whose operand is unaligned and in caching-inhibited storage.
#[test]
fn a_misaligned_inhibited_store_whose_first_page_c_write_waits_for_a_store_keeps_no_first_page_r() {
    // Page 4's entry has R = C = 0. The stb writes byte 15 of that entry
    // with its value unchanged. The C write of page 4 waits for the stb,
    // so the batch ends ahead of the sth before it reaches page 5.
    let page = PAGE_RA | 0b10;
    let mut mem = memory(&[stb(6, 0, 7), sth(4, 0xFFF, 5)], page);
    let mut unit = kernel_unit();
    unit.state_mut()
        .set_gpr(6, u64::from(page.to_be_bytes()[PTE_C_BYTE - 8]));
    unit.state_mut().set_gpr(7, entry_byte_ea(PAGE, PTE_C_BYTE));
    run_to_alignment(&mut unit, &mut mem);
    assert_r_byte_unchanged(&mem, PAGE, page);
}

/// [CBE-Handbook p:90 s:4.2.6.8] Footnote 4: a misaligned caching-inhibited store is speculative, so its table lookup is not performed when it takes the alignment interrupt.
/// [CBE-Handbook p:253 s:9.5.8] The PPE takes the alignment interrupt for a load or store whose operand is unaligned and in caching-inhibited storage.
#[test]
fn a_misaligned_inhibited_store_whose_next_page_aliases_a_stored_page_keeps_no_r_write() {
    // Page 6 maps page 5's real page, caching-inhibited with R and C set.
    // The aligned stw writes through page 6. The translation of page 5
    // then aliases page 6, so the batch ends ahead of the sth.
    let page = PAGE_RA | PTE_C | 0b10;
    let mut mem = memory(&[stw(8, 0, 9), sth(4, 0xFFF, 5)], page);
    map_page(&mut mem, ALIAS, NEXT_RA | PTE_I | PTE_R | PTE_C | 0b10);
    let mut unit = kernel_unit();
    unit.state_mut().set_gpr(9, SEGMENT + (ALIAS << 12));
    run_to_alignment(&mut unit, &mut mem);
    assert_r_byte_unchanged(&mem, PAGE, page);
    assert_r_byte_unchanged(&mem, NEXT, NEXT_D1);
}

/// [CBE-Handbook p:90 s:4.2.6.8] Footnote 4: a misaligned caching-inhibited store is speculative, so its table lookup is not performed when it takes the alignment interrupt.
/// [CBE-Handbook p:290 s:9.10] A caching-inhibited store's alignment interrupt with C = 1 comes before the page-protection data storage interrupt.
#[test]
fn a_misaligned_store_into_a_read_only_inhibited_page_with_c_1_whose_batch_ends_keeps_no_first_page_r(
) {
    // Page 5 is read-only and caching-inhibited with C = 1, so the store
    // meets its load translation. The stb writes byte 14 of page 5's entry
    // with its value unchanged. The search of page 5 waits for the stb, so
    // the batch ends ahead of the sth.
    let next = NEXT_RA | PTE_I | PTE_C | 0b11;
    let page = PAGE_RA | PTE_C | 0b10;
    let mut mem = memory(&[stb(6, 0, 7), sth(4, 0xFFF, 5)], page);
    map_page(&mut mem, NEXT, next);
    let mut unit = kernel_unit();
    unit.state_mut()
        .set_gpr(6, u64::from(next.to_be_bytes()[PTE_R_BYTE - 8]));
    unit.state_mut().set_gpr(7, entry_byte_ea(NEXT, PTE_R_BYTE));
    run_to_alignment(&mut unit, &mut mem);
    assert_r_byte_unchanged(&mem, PAGE, page);
    assert_r_byte_unchanged(&mem, NEXT, next);
}
