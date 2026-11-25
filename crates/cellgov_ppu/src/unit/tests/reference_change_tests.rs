//! A translating thread's table search writes the R and C bits of the
//! entry it matches, as effects of the access, and the bits read back
//! from the page table's storage.

use crate::mmu::Partition;
use crate::supervisor::{SlbEntry, SupervisorState};
use crate::*;
use cellgov_effects::Effect;
use cellgov_event::UnitId;
use cellgov_exec::{ExecutionContext, ExecutionStepResult, ExecutionUnit, YieldReason};
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory, PageSize, Region, RegionAccess};
use cellgov_ps3_abi::hw::ppu_interrupt as irq;
use cellgov_ps3_abi::hw::ppu_mmu::{PTE_BYTES, PTE_C, PTE_G, PTE_R};
use cellgov_ps3_abi::hw::ppu_supervisor::{MSR_DR, MSR_IR, MSR_SF, SLB_VALID};
use cellgov_time::Budget;

#[path = "reference_change_order_tests.rs"]
mod order;

/// The page table: 2^11 groups from 256 KB to the end of the memory.
const HTAB: u64 = 0x4_0000;
/// The segment the SLB maps: EA 0xC000_0000, VSID 0x123.
const SEGMENT: u64 = 0xC000_0000;
const VSID: u64 = 0x123;
/// Page 0 holds the code, at real 0x3000.
const CODE_RA: u64 = 0x3000;
/// Page 1 is read/write data at real 0x5000.
const DATA_EA: u64 = SEGMENT + 0x1000;
const DATA_RA: u64 = 0x5000;
/// Page 2 is read/write data at real 0x7000.
const FAR_RA: u64 = 0x7000;
/// Page 4 is read only at real 0x6000.
const RO_EA: u64 = SEGMENT + 0x4000;

fn lwz(rt: u32, d: u16, ra: u32) -> u32 {
    (32 << 26) | (rt << 21) | (ra << 16) | u32::from(d)
}

fn stw(rs: u32, d: u16, ra: u32) -> u32 {
    (36 << 26) | (rs << 21) | (ra << 16) | u32::from(d)
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
    HTAB + ((VSID ^ index) & 0x7FF) * 128
}

/// Writes the entry for page `index` with doubleword 1 `d1`.
fn map_page(mem: &mut GuestMemory, index: u64, d1: u64) {
    let d0 = (VSID << 12) | 1;
    put(mem, entry(index), &d0.to_be_bytes());
    put(mem, entry(index) + 8, &d1.to_be_bytes());
}

/// The 16 bytes of the entry for page `index`, read from storage.
fn entry_bytes(mem: &GuestMemory, index: u64) -> [u8; PTE_BYTES] {
    let range = ByteRange::new(GuestAddr::new(entry(index)), PTE_BYTES as u64).unwrap();
    mem.read(range).unwrap().as_ref().try_into().unwrap()
}

/// Doubleword 1 of the entry for page `index`, read from storage.
fn d1(mem: &GuestMemory, index: u64) -> u64 {
    u64::from_be_bytes(entry_bytes(mem, index)[8..].try_into().unwrap())
}

/// 512 KB of memory that holds:
///
/// - the page table, with R and C clear in every entry;
/// - `code` on page 0;
/// - the entries of pages 1, 2 and 4.
fn memory(code: &[u32]) -> GuestMemory {
    memory_over(GuestMemory::new(0x8_0000), code)
}

/// [`memory`] over `mem`, which holds at least its first 512 KB.
fn memory_over(mut mem: GuestMemory, code: &[u32]) -> GuestMemory {
    map_page(&mut mem, 0, CODE_RA | 0b10);
    map_page(&mut mem, 1, DATA_RA | 0b10);
    map_page(&mut mem, 2, FAR_RA | 0b10);
    map_page(&mut mem, 4, 0x6000 | 0b11);
    for (i, word) in code.iter().enumerate() {
        put(&mut mem, CODE_RA + 4 * i as u64, &word.to_be_bytes());
    }
    mem
}

/// A unit at the segment's first word, with:
///
/// - relocation on;
/// - SLB entry 0 over the segment;
/// - the page table at [`HTAB`].
fn kernel_unit() -> PpuExecutionUnit {
    let mut sup = SupervisorState::new(MSR_SF | MSR_IR | MSR_DR);
    sup.slb[0] = SlbEntry {
        esid_v: SEGMENT | SLB_VALID,
        vsid_flags: VSID << 12,
    };
    sup.partition = Partition::new(HTAB, 1 << 62, 0, Partition::INITIAL.lb()).unwrap();
    let mut unit = PpuExecutionUnit::new(UnitId::new(0));
    unit.state_mut().set_supervisor(Some(sup));
    unit.state_mut().pc = SEGMENT;
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

/// The write intents, in order, as start and bytes.
fn write_intents(effects: &[Effect]) -> Vec<(u64, Vec<u8>)> {
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

/// The starts of the read intents in the page table, in order.
fn table_reads(effects: &[Effect]) -> Vec<u64> {
    effects
        .iter()
        .filter_map(|e| match e {
            Effect::SharedReadIntent { range, .. } if in_table(range.start().raw()) => {
                Some(range.start().raw())
            }
            _ => None,
        })
        .collect()
}

/// [`run`] with its write intents committed to `mem` in order.
///
/// It returns the write intents in the form [`write_intents`] gives.
fn run_and_commit(
    unit: &mut PpuExecutionUnit,
    mem: &mut GuestMemory,
    budget: u64,
) -> (ExecutionStepResult, Vec<(u64, Vec<u8>)>) {
    let (r, effects) = run(unit, mem, budget);
    let writes = write_intents(&effects);
    for (at, bytes) in &writes {
        put(mem, *at, bytes);
    }
    (r, writes)
}

/// Whether `at` lies in the page table.
fn in_table(at: u64) -> bool {
    (HTAB..HTAB + 0x4_0000).contains(&at)
}

/// [CBE-Handbook p:90 s:4.2.6.8] A table lookup that matches an entry with R = 0 sets R.
/// [PPC-Book3 p:44 s:4.9] Figure 29: an in-order fetch or load sets R and leaves C.
/// [PPC-Book3 p:2 s:1.3.2] When the processor writes a table field, no other defined field changes.
#[test]
fn a_load_and_its_fetch_set_r_and_leave_c_and_every_other_entry_byte() {
    let mut mem = memory(&[lwz(4, 0, 5)]);
    let before = [entry_bytes(&mem, 0), entry_bytes(&mem, 1)];
    let mut unit = kernel_unit();
    unit.state_mut().set_gpr(5, DATA_EA);
    let (r, _) = run_and_commit(&mut unit, &mut mem, 1);
    assert_eq!(r.yield_reason, YieldReason::BudgetExhausted);
    for (index, old) in [0, 1].into_iter().zip(before) {
        let mut want = old;
        want[14] |= 0x01;
        assert_eq!(entry_bytes(&mem, index), want, "page {index}");
        assert_eq!(d1(&mem, index) & PTE_C, 0, "page {index}");
    }
}

/// [PPC-Book3 p:44 s:4.9] Figure 29: an in-order ordinary store sets R and C.
#[test]
fn a_store_sets_r_and_c_of_its_entry() {
    let mut mem = memory(&[stw(4, 0, 5)]);
    let mut unit = kernel_unit();
    unit.state_mut().set_gpr(5, DATA_EA);
    let (r, _) = run_and_commit(&mut unit, &mut mem, 1);
    assert_eq!(r.yield_reason, YieldReason::BudgetExhausted);
    assert_eq!(d1(&mem, 1), DATA_RA | PTE_R | PTE_C | 0b10);
}

#[test]
fn a_store_through_a_translation_a_load_cached_in_the_batch_sets_c() {
    let mut mem = memory(&[lwz(4, 0, 5), stw(4, 4, 5)]);
    let mut unit = kernel_unit();
    unit.state_mut().set_gpr(5, DATA_EA);
    let (r, _) = run_and_commit(&mut unit, &mut mem, 2);
    assert_eq!(r.yield_reason, YieldReason::BudgetExhausted);
    assert_eq!(unit.retired_instructions(), 2, "both ran in one batch");
    assert_eq!(d1(&mem, 1) & (PTE_R | PTE_C), PTE_R | PTE_C);
}

/// [CBE-Handbook p:90 s:4.2.6.8] The PPE sets R regardless of any storage interrupt, and does not set C for an access that takes a storage exception.
#[test]
fn a_store_the_page_protection_refuses_sets_r_and_not_c() {
    let mut mem = memory(&[stw(4, 0, 5)]);
    let mut unit = kernel_unit();
    unit.state_mut().set_gpr(5, RO_EA);
    let (r, _) = run_and_commit(&mut unit, &mut mem, 1);
    let record = r.local_diagnostics.interrupt.expect("an interrupt");
    assert_eq!(
        (record.vector, record.dsisr),
        (
            irq::VECTOR_DATA_STORAGE,
            Some(irq::DSISR_PROTECTION | irq::DSISR_STORE)
        )
    );
    assert_eq!(d1(&mem, 4) & (PTE_R | PTE_C), PTE_R);
}

/// [PPC-Book3 p:43 s:4.9] The pieces of an operand that crosses a page boundary are separate accesses for R and C.
#[test]
fn a_store_across_two_pages_sets_r_and_c_of_each_pages_entry() {
    let mut mem = memory(&[stw(4, 0xFFE, 5)]);
    let mut unit = kernel_unit();
    unit.state_mut().set_gpr(5, DATA_EA);
    let (r, _) = run_and_commit(&mut unit, &mut mem, 1);
    assert_eq!(r.yield_reason, YieldReason::BudgetExhausted);
    assert_eq!(d1(&mem, 1) & (PTE_R | PTE_C), PTE_R | PTE_C);
    assert_eq!(d1(&mem, 2) & (PTE_R | PTE_C), PTE_R | PTE_C);
}

/// [PPC-Book3 p:43 s:4.9] The processor writes R or C only when the new value differs from the old.
#[test]
fn a_set_bit_is_not_written_and_each_write_is_its_own_byte() {
    // Page 0 and page 1 have R = 1; page 1 has C = 0.
    let mut mem = memory(&[lwz(4, 0, 5), stw(4, 4, 5)]);
    map_page(&mut mem, 0, CODE_RA | PTE_R | 0b10);
    map_page(&mut mem, 1, DATA_RA | PTE_R | 0b10);
    let mut unit = kernel_unit();
    unit.state_mut().set_gpr(5, DATA_EA);
    let (_, writes) = run_and_commit(&mut unit, &mut mem, 2);
    let table: Vec<_> = writes.into_iter().filter(|(at, _)| in_table(*at)).collect();
    assert_eq!(table, vec![(entry(1) + 15, vec![0x80 | 0b10])]);
}

/// The order is a CellGov convention. The commit pipeline makes all
/// writes of a batch visible at one epoch boundary. So no other unit,
/// DMA transfer or hypercall sees their order.
#[test]
fn the_entry_writes_publish_ahead_of_the_blocks_stores() {
    let mut mem = memory(&[stw(4, 0, 5), lwz(6, 0, 7)]);
    let mut unit = kernel_unit();
    unit.state_mut().set_gpr(5, DATA_EA);
    unit.state_mut().set_gpr(7, SEGMENT + 0x2000);
    let (r, writes) = run_and_commit(&mut unit, &mut mem, 2);
    assert_eq!(r.yield_reason, YieldReason::BudgetExhausted);
    let starts: Vec<u64> = writes.iter().map(|(at, _)| *at).collect();
    assert_eq!(
        starts,
        vec![
            entry(0) + 14,
            entry(1) + 14,
            entry(1) + 15,
            entry(2) + 14,
            DATA_RA
        ]
    );
}

/// [CBE-Handbook p:90 s:4.2.6.8] The PPE performs every store, its page table entry updates included, in order.
#[test]
fn a_search_whose_byte_a_buffered_store_writes_waits_for_that_store() {
    // Page 3 maps the page of real memory that holds the entries of
    // pages 0, 1 and 3. Its own entry has R and C set, so a store through
    // it needs no entry write. The stb writes byte 14 of page 1's entry
    // with R = 0 and reserved bit 53 set.
    let page_of_entries = entry(1) & !0xFFF;
    let mut mem = memory(&[stb(6, 0, 7), lwz(4, 0, 5)]);
    map_page(&mut mem, 3, page_of_entries | PTE_R | PTE_C | 0b10);
    let mut unit = kernel_unit();
    unit.state_mut().set_gpr(5, DATA_EA);
    unit.state_mut().set_gpr(6, 0x54);
    unit.state_mut()
        .set_gpr(7, SEGMENT + 0x3000 + (entry(1) + 14 - page_of_entries));
    let (r, writes) = run_and_commit(&mut unit, &mut mem, 2);
    assert_eq!(r.yield_reason, YieldReason::BudgetExhausted);
    assert_eq!(
        unit.retired_instructions(),
        1,
        "the load waits for the store"
    );
    assert!(
        writes.contains(&(entry(1) + 14, vec![0x54])),
        "the stb: {writes:x?}"
    );
    let (r, _) = run_and_commit(&mut unit, &mut mem, 1);
    assert_eq!(r.yield_reason, YieldReason::BudgetExhausted);
    assert_eq!(entry_bytes(&mem, 1)[14], 0x55, "R set over the stored byte");
}

/// [CBE-Handbook p:90 s:4.2.6.8] The PPE performs every store, its page table entry updates included, in order.
#[test]
fn a_fetch_whose_r_byte_a_buffered_store_writes_waits_for_that_store() {
    // As above, page 3 maps the entries of pages 0, 1 and 3. The stb
    // writes byte 14 of page 1's entry, and the branch then goes to
    // page 1, whose fetch needs R.
    const NOP: u32 = 0x6000_0000;
    const B_TO_PAGE_1: u32 = 0x4800_0FFC;
    let page_of_entries = entry(1) & !0xFFF;
    let mut mem = memory(&[stb(6, 0, 7), B_TO_PAGE_1]);
    map_page(&mut mem, 3, page_of_entries | PTE_R | PTE_C | 0b10);
    put(&mut mem, DATA_RA, &NOP.to_be_bytes());
    let mut unit = kernel_unit();
    unit.state_mut().set_gpr(6, 0x54);
    unit.state_mut()
        .set_gpr(7, SEGMENT + 0x3000 + (entry(1) + 14 - page_of_entries));
    let (r, writes) = run_and_commit(&mut unit, &mut mem, 3);
    assert_eq!(r.yield_reason, YieldReason::BudgetExhausted);
    assert!(r.fault.is_none(), "{:?}", r.fault);
    assert_eq!(
        unit.retired_instructions(),
        2,
        "the fetch waits for the store"
    );
    assert_eq!(unit.state().pc, DATA_EA);
    let at_r: Vec<_> = writes
        .iter()
        .filter(|(at, _)| *at == entry(1) + 14)
        .collect();
    assert_eq!(at_r, vec![&(entry(1) + 14, vec![0x54])], "only the stb");
    let (r, _) = run_and_commit(&mut unit, &mut mem, 1);
    assert_eq!(r.yield_reason, YieldReason::BudgetExhausted);
    assert_eq!(unit.retired_instructions(), 3);
    assert_eq!(entry_bytes(&mem, 1)[14], 0x55, "R set over the stored byte");
}

/// [CBE-Handbook p:90 s:4.2.6.8] The PPE performs every store, its page table entry updates included, in order.
#[test]
fn a_change_write_whose_byte_a_buffered_store_writes_waits_for_that_store() {
    // The stb writes byte 15 of page 1's entry with C = 0, M = 1 and
    // PP 10, and the stw to page 1 then needs C.
    let page_of_entries = entry(1) & !0xFFF;
    let mut mem = memory(&[stb(6, 0, 7), stw(4, 0, 5)]);
    map_page(&mut mem, 3, page_of_entries | PTE_R | PTE_C | 0b10);
    let mut unit = kernel_unit();
    unit.state_mut().set_gpr(5, DATA_EA);
    unit.state_mut().set_gpr(6, 0x12);
    unit.state_mut()
        .set_gpr(7, SEGMENT + 0x3000 + (entry(1) + 15 - page_of_entries));
    let (r, writes) = run_and_commit(&mut unit, &mut mem, 2);
    assert_eq!(r.yield_reason, YieldReason::BudgetExhausted);
    assert_eq!(unit.retired_instructions(), 1, "the stw waits for the stb");
    let at_c: Vec<_> = writes
        .iter()
        .filter(|(at, _)| *at == entry(1) + 15)
        .collect();
    assert_eq!(at_c, vec![&(entry(1) + 15, vec![0x12])], "only the stb");
    let (r, _) = run_and_commit(&mut unit, &mut mem, 1);
    assert_eq!(r.yield_reason, YieldReason::BudgetExhausted);
    assert_eq!(unit.retired_instructions(), 2);
    assert_eq!(entry_bytes(&mem, 1)[15], 0x92, "C set over the stored byte");
}

#[test]
fn a_buffered_store_to_the_byte_ahead_of_r_does_not_hold_the_search() {
    // The stb writes byte 13 of page 1's entry with its own value, so the
    // load's R write to byte 14 needs no wait.
    let page_of_entries = entry(1) & !0xFFF;
    let mut mem = memory(&[stb(6, 0, 7), lwz(4, 0, 5)]);
    map_page(&mut mem, 3, page_of_entries | PTE_R | PTE_C | 0b10);
    let mut unit = kernel_unit();
    unit.state_mut().set_gpr(5, DATA_EA);
    unit.state_mut().set_gpr(6, 0);
    unit.state_mut()
        .set_gpr(7, SEGMENT + 0x3000 + (entry(1) + 13 - page_of_entries));
    let (r, writes) = run_and_commit(&mut unit, &mut mem, 2);
    assert_eq!(r.yield_reason, YieldReason::BudgetExhausted);
    assert_eq!(unit.retired_instructions(), 2, "the load does not wait");
    let table: Vec<_> = writes.into_iter().filter(|(at, _)| in_table(*at)).collect();
    assert_eq!(
        table,
        vec![
            (entry(0) + 14, vec![0x31]),
            (entry(1) + 14, vec![0x51]),
            (entry(1) + 13, vec![0x00])
        ]
    );
}

/// [CBE-Handbook p:90 s:4.2.6.8] The PPE sets R regardless of any storage interrupt.
/// [PPC-Book3 p:68 s:5.5] A fetch from guarded storage takes the instruction storage interrupt.
#[test]
fn a_fetch_its_entry_refuses_writes_r_and_publishes_the_group_it_read() {
    let mut mem = memory(&[lwz(4, 0, 5)]);
    map_page(&mut mem, 0, CODE_RA | PTE_G | 0b10);
    let mut unit = kernel_unit();
    let (r, effects) = run(&mut unit, &mem, 1);
    let record = r.local_diagnostics.interrupt.expect("an interrupt");
    assert_eq!(record.vector, irq::VECTOR_INSTRUCTION_STORAGE);
    assert_eq!(table_reads(&effects), vec![entry(0)]);
    assert_eq!(write_intents(&effects), vec![(entry(0) + 14, vec![0x31])]);
}

#[test]
fn a_search_whose_entry_has_r_set_publishes_the_group_it_read_and_writes_nothing() {
    let mut mem = memory(&[lwz(4, 0, 5)]);
    map_page(&mut mem, 0, CODE_RA | PTE_R | 0b10);
    map_page(&mut mem, 1, DATA_RA | PTE_R | 0b10);
    let mut unit = kernel_unit_at_gpr5(DATA_EA);
    let (r, effects) = run(&mut unit, &mem, 1);
    assert_eq!(r.yield_reason, YieldReason::BudgetExhausted);
    assert_eq!(table_reads(&effects), vec![entry(0), entry(1)]);
    let writes = write_intents(&effects);
    assert!(writes.is_empty(), "no entry write: {writes:x?}");
}

/// [`kernel_unit`] with r5 at `ea`.
fn kernel_unit_at_gpr5(ea: u64) -> PpuExecutionUnit {
    let mut unit = kernel_unit();
    unit.state_mut().set_gpr(5, ea);
    unit
}

/// [CBE-Handbook p:90 s:4.2.6.8] A store's lookup does not set C when the access takes a storage exception.
#[test]
fn a_store_to_a_writable_page_a_region_refuses_takes_the_c_set_interrupt_and_writes_no_c() {
    // Page 5 maps a region that refuses writes. The store takes the
    // region's refusal whether the entry's C is 0 or 1.
    const ZERO_RA: u64 = 0x10_0000;
    let stop = |c: u64| {
        let ram = Region::new(0, 0x8_0000, "ram", PageSize::Page64K);
        let zero = Region::with_access(
            ZERO_RA,
            0x1000,
            "zero",
            PageSize::Page4K,
            RegionAccess::ReservedZeroReadable,
        );
        let base = GuestMemory::from_regions(vec![ram, zero]).unwrap();
        let mut mem = memory_over(base, &[stw(4, 0, 5)]);
        map_page(&mut mem, 5, ZERO_RA | c | 0b10);
        let mut unit = kernel_unit_at_gpr5(SEGMENT + 0x5000);
        let (r, writes) = run_and_commit(&mut unit, &mut mem, 1);
        (r.local_diagnostics.interrupt, writes)
    };
    let (record, writes) = stop(0);
    let (record_c, _) = stop(PTE_C);
    assert_eq!(
        record.map(|i| i.vector),
        Some(irq::VECTOR_DATA_STORAGE),
        "{record:?}"
    );
    assert_eq!(record, record_c);
    assert_eq!(
        writes,
        vec![(entry(0) + 14, vec![0x31]), (entry(5) + 14, vec![0x01])],
        "the fetch's and the store's R writes, and no C write"
    );
}
