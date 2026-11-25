//! A store that crosses from a writable page whose entry has C = 0 into
//! a page or segment that refuses it takes the data storage or data
//! segment interrupt. It writes R to the entry of each page it searched
//! and writes C to no entry.

use crate::mmu::Partition;
use crate::supervisor::{SlbEntry, SupervisorState};
use crate::*;
use cellgov_effects::Effect;
use cellgov_event::UnitId;
use cellgov_exec::{ExecutionContext, ExecutionStepResult, ExecutionUnit};
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory, PageSize, Region, RegionAccess};
use cellgov_ps3_abi::hw::ppu_interrupt as irq;
use cellgov_ps3_abi::hw::ppu_mmu::{PTE_C, PTE_C_BYTE, PTE_R, PTE_R_BYTE};
use cellgov_ps3_abi::hw::ppu_supervisor::{MSR_DR, MSR_IR, MSR_ME, MSR_SF, SLB_VALID};
use cellgov_time::Budget;

/// The page table: 2^11 groups from 256 KB to the end of the RAM.
const HTAB: u64 = 0x4_0000;
/// The bytes of RAM, from real address 0.
const RAM: usize = 0x8_0000;
/// The segment that the SLB maps, and its VSID.
const SEGMENT: u64 = 0xC000_0000;
const VSID: u64 = 0x123;
/// The real address of page 0, which holds the code.
const CODE_RA: u64 = 0x3000;
/// Doubleword 1 of page 0's entry: read/write with R = C = 0.
const CODE_D1: u64 = CODE_RA | 0b10;
/// Page 4, where each store starts, and doubleword 1 of its entry:
/// read/write and cacheable with R = C = 0.
const PAGE: u64 = 4;
const PAGE_EA: u64 = SEGMENT + 0x4000;
const PAGE_D1: u64 = 0x6000 | 0b10;
/// Page 5, after page 4, and its real address.
const NEXT: u64 = 5;
const NEXT_RA: u64 = 0x7000;
/// The real address of a region that refuses writes, past the RAM.
const ZERO_RA: u64 = 0x10_0000;
/// Bytes in a group of page table entries.
const GROUP: u64 = 128;

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
    HTAB + ((VSID ^ index) & 0x7FF) * GROUP
}

/// Writes the entry for page `index` with doubleword 1 `d1`.
fn map_page(mem: &mut GuestMemory, index: u64, d1: u64) {
    let d0 = (VSID << 12) | 1;
    put(mem, entry(index), &d0.to_be_bytes());
    put(mem, entry(index) + 8, &d1.to_be_bytes());
}

/// RAM from real address 0, and a 4 KB region at [`ZERO_RA`] that reads
/// as zero and refuses writes.
fn ram_and_zero_region() -> GuestMemory {
    let ram = Region::new(0, RAM, "ram", PageSize::Page64K);
    let zero = Region::with_access(
        ZERO_RA,
        0x1000,
        "zero",
        PageSize::Page4K,
        RegionAccess::ReservedZeroReadable,
    );
    GuestMemory::from_regions(vec![ram, zero]).unwrap()
}

/// [`ram_and_zero_region`] with:
///
/// - the page table;
/// - `code` on page 0, whose entry has doubleword 1 [`CODE_D1`];
/// - the entry of page 4 with doubleword 1 [`PAGE_D1`];
/// - the entry of page 5 with doubleword 1 `next`, when `next` is `Some`.
fn memory(code: &[u32], next: Option<u64>) -> GuestMemory {
    let mut mem = ram_and_zero_region();
    map_page(&mut mem, 0, CODE_D1);
    map_page(&mut mem, PAGE, PAGE_D1);
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

/// The vector and DSISR of the interrupt the run took.
fn interrupt(r: &ExecutionStepResult) -> Option<(u64, Option<u32>)> {
    r.local_diagnostics.interrupt.map(|i| (i.vector, i.dsisr))
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

/// [CBE-Handbook p:90 s:4.2.6.8] The MMU does not update C when the store takes a storage exception, and it sets R regardless of storage interrupts.
/// [PPC-Book3 p:43 s:4.9] A Data Storage exception of a store is a case where C may be set, so the PPE handbook decides.
#[test]
fn a_store_into_a_next_page_the_table_refuses_writes_r_and_no_c_to_the_first_page() {
    // (page 5's doubleword 1, the DSISR). Page 5 has no entry, or it is
    // read-only. The stw starts two bytes before the end of page 4.
    let cases = [
        (None, irq::DSISR_NOT_FOUND | irq::DSISR_STORE),
        (
            Some(NEXT_RA | 0b11),
            irq::DSISR_PROTECTION | irq::DSISR_STORE,
        ),
    ];
    for (next, dsisr) in cases {
        let mem = memory(&[stw(4, 0xFFE, 5)], next);
        let mut unit = kernel_unit();
        let (r, effects) = run(&mut unit, &mem, 1);
        let case = format!("page 5 {next:x?}");
        assert_eq!(
            interrupt(&r),
            Some((irq::VECTOR_DATA_STORAGE, Some(dsisr))),
            "{case}"
        );
        assert_eq!(
            entry_byte_writes(&effects, PAGE, PTE_R_BYTE),
            vec![r_byte(PAGE_D1)],
            "{case}: page 4 R"
        );
        assert_eq!(
            entry_byte_writes(&effects, PAGE, PTE_C_BYTE),
            Vec::<u8>::new(),
            "{case}: page 4 C"
        );
        assert_eq!(
            entry_byte_writes(&effects, NEXT, PTE_R_BYTE),
            next.map(r_byte).into_iter().collect::<Vec<_>>(),
            "{case}: page 5 R"
        );
        assert_eq!(
            entry_byte_writes(&effects, NEXT, PTE_C_BYTE),
            Vec::<u8>::new(),
            "{case}: page 5 C"
        );
    }
}

/// [CBE-Handbook p:90 s:4.2.6.8] The MMU does not update C when the store takes a storage exception.
/// [PPC-Book3 p:43 s:4.9] The processor writes R or C only when the new value differs from the old.
#[test]
fn a_store_into_a_next_page_the_table_refuses_writes_no_c_to_a_first_page_whose_r_is_set() {
    // Page 4's entry has R = 1 and C = 0, so the C write is the first
    // entry write the stw makes. Page 5 has no entry.
    let page = PAGE_D1 | PTE_R;
    let mut mem = memory(&[stw(4, 0xFFE, 5)], None);
    map_page(&mut mem, PAGE, page);
    let mut unit = kernel_unit();
    let (r, effects) = run(&mut unit, &mem, 1);
    assert_eq!(
        interrupt(&r),
        Some((
            irq::VECTOR_DATA_STORAGE,
            Some(irq::DSISR_NOT_FOUND | irq::DSISR_STORE)
        ))
    );
    assert_eq!(
        entry_byte_writes(&effects, PAGE, PTE_R_BYTE),
        Vec::<u8>::new(),
        "page 4 R"
    );
    assert_eq!(
        entry_byte_writes(&effects, PAGE, PTE_C_BYTE),
        Vec::<u8>::new(),
        "page 4 C"
    );
}

/// [CBE-Handbook p:90 s:4.2.6.8] The MMU does not update C when the store takes a storage exception, and it sets R regardless of storage interrupts.
#[test]
fn a_store_into_a_next_page_whose_region_refuses_writes_writes_r_and_no_c_to_either_page() {
    // Page 5 is read/write with C = 0 over a region that refuses writes.
    // Its translation succeeds, and the region refuses the store.
    let next = ZERO_RA | 0b10;
    let mem = memory(&[stw(4, 0xFFE, 5)], Some(next));
    let mut unit = kernel_unit();
    let (r, effects) = run(&mut unit, &mem, 1);
    assert_eq!(
        interrupt(&r).map(|(vector, _)| vector),
        Some(irq::VECTOR_DATA_STORAGE)
    );
    assert_eq!(
        entry_byte_writes(&effects, PAGE, PTE_R_BYTE),
        vec![r_byte(PAGE_D1)],
        "page 4 R"
    );
    assert_eq!(
        entry_byte_writes(&effects, PAGE, PTE_C_BYTE),
        Vec::<u8>::new(),
        "page 4 C"
    );
    assert_eq!(
        entry_byte_writes(&effects, NEXT, PTE_R_BYTE),
        vec![r_byte(next)],
        "page 5 R"
    );
    assert_eq!(
        entry_byte_writes(&effects, NEXT, PTE_C_BYTE),
        Vec::<u8>::new(),
        "page 5 C"
    );
}

/// [CBE-Handbook p:90 s:4.2.6.8] A store's table lookup sets C = 0 to 1, and the MMU does not update C when the store takes a storage exception.
#[test]
fn an_earlier_stores_c_write_stays_when_a_crossing_store_takes_the_data_storage_interrupt() {
    // The first stw retires and writes page 4's C. The second crosses into
    // page 5, which has no entry, and the batch ends ahead of it.
    let mem = memory(&[stw(4, 0, 5), stw(4, 0xFFE, 5)], None);
    let mut unit = kernel_unit();
    let (_, effects) = run(&mut unit, &mem, 2);
    assert_eq!(
        unit.retired_instructions(),
        1,
        "the second stw does not retire"
    );
    assert_eq!(
        entry_byte_writes(&effects, PAGE, PTE_C_BYTE),
        vec![c_byte(PAGE_D1)]
    );
}

/// [CBE-Handbook p:90 s:4.2.6.8] The MMU does not update C when the store takes a storage exception, and it sets R regardless of storage interrupts.
/// [CBE-Handbook p:90 s:4.2.6.8] The PPE performs every store, its page table entry updates included, in order.
#[test]
fn a_crossing_store_whose_batch_ends_ahead_of_its_next_page_publishes_no_c_write() {
    // Page 3 maps the page of real memory that holds the entries of pages
    // 0, 3, 4 and 5. Its own entry has R and C set, so a store through it
    // needs no entry write. The stb writes byte 14 of page 5's entry with
    // its value unchanged. The search of page 5 waits for the stb, so the
    // batch ends ahead of the stw.
    let next = NEXT_RA | 0b11;
    let page_of_entries = entry(NEXT) & !0xFFF;
    let mut mem = memory(&[stb(6, 0, 7), stw(4, 0xFFE, 5)], Some(next));
    map_page(&mut mem, 3, page_of_entries | PTE_R | PTE_C | 0b10);
    let mut unit = kernel_unit();
    unit.state_mut()
        .set_gpr(6, u64::from(next.to_be_bytes()[PTE_R_BYTE - 8]));
    unit.state_mut()
        .set_gpr(7, SEGMENT + 0x3000 + (entry(NEXT) + 14 - page_of_entries));
    let (_, effects) = run(&mut unit, &mem, 2);
    assert_eq!(unit.retired_instructions(), 1, "the stw waits for the stb");
    assert_eq!(
        entry_byte_writes(&effects, PAGE, PTE_R_BYTE),
        vec![r_byte(PAGE_D1)],
        "batch 1: page 4 R"
    );
    assert_eq!(
        entry_byte_writes(&effects, PAGE, PTE_C_BYTE),
        Vec::<u8>::new(),
        "batch 1: page 4 C"
    );
    for e in &effects {
        if let Effect::SharedWriteIntent { range, bytes, .. } = e {
            put(&mut mem, range.start().raw(), bytes.bytes());
        }
    }
    let (r, effects) = run(&mut unit, &mem, 1);
    assert_eq!(
        interrupt(&r),
        Some((
            irq::VECTOR_DATA_STORAGE,
            Some(irq::DSISR_PROTECTION | irq::DSISR_STORE)
        )),
        "batch 2"
    );
    assert_eq!(
        entry_byte_writes(&effects, PAGE, PTE_C_BYTE),
        Vec::<u8>::new(),
        "batch 2: page 4 C"
    );
    assert_eq!(
        entry_byte_writes(&effects, NEXT, PTE_R_BYTE),
        vec![r_byte(next)],
        "batch 2: page 5 R"
    );
}

/// [CBE-Handbook p:90 s:4.2.6.8] The MMU does not update C when the store takes a storage exception, and it sets R regardless of storage interrupts.
/// [PPC-Book3 p:26 s:4.2.1] A storage exception is one that causes, among others, a Data Segment interrupt.
#[test]
fn a_store_into_a_next_segment_the_slb_does_not_map_writes_r_and_no_c_to_the_first_page() {
    // The stw starts two bytes before the end of the segment's last page.
    // The SLB maps no entry for the next segment.
    const LAST: u64 = 0xFFFF;
    let mut mem = memory(&[stw(4, 0xFFE, 5)], None);
    // [PPC-Book3 p:38 s:4.5.3] The AVPN of a 4 KB page holds the five high bits of its page index after the VSID.
    let d0 = (VSID << 12) | ((LAST >> 11) << 7) | 1;
    put(&mut mem, entry(LAST), &d0.to_be_bytes());
    put(&mut mem, entry(LAST) + 8, &PAGE_D1.to_be_bytes());
    let mut unit = kernel_unit();
    unit.state_mut().set_gpr(5, SEGMENT + (LAST << 12));
    let (r, effects) = run(&mut unit, &mem, 1);
    assert_eq!(
        interrupt(&r),
        Some((
            irq::VECTOR_DATA_SEGMENT,
            Some(irq::DSISR_SEGMENT | irq::DSISR_STORE)
        ))
    );
    assert_eq!(
        entry_byte_writes(&effects, LAST, PTE_R_BYTE),
        vec![r_byte(PAGE_D1)],
        "last page R"
    );
    assert_eq!(
        entry_byte_writes(&effects, LAST, PTE_C_BYTE),
        Vec::<u8>::new(),
        "last page C"
    );
}
