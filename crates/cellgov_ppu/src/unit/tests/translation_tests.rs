//! A thread with supervisor registers runs at effective addresses and
//! publishes real ones: through real addressing mode, and through its SLB
//! and the page table, with the interrupts translation raises.

use crate::fault_codes::{FAULT_CHECKSTOP, FAULT_MACHINE_CHECK, FAULT_UNMODELLED};
use crate::mmu::Partition;
use crate::supervisor::{SlbEntry, SupervisorState};
use crate::*;
use cellgov_effects::Effect;
use cellgov_event::UnitId;
use cellgov_exec::{
    ExecutionContext, ExecutionStepResult, ExecutionUnit, InterruptRecord, YieldReason,
};
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_ps3_abi::hw::ppu_interrupt as irq;
use cellgov_ps3_abi::hw::ppu_supervisor::{MSR_DR, MSR_IR, MSR_ME, MSR_SF, SLB_VALID};
use cellgov_time::Budget;

/// The page table: 2^11 groups from 256 KB to the end of the memory.
const HTAB: u64 = 0x4_0000;
/// The segment the SLB maps: EA 0xC000_0000, VSID 0x123.
const SEGMENT: u64 = 0xC000_0000;
const VSID: u64 = 0x123;
/// Code page: EA `SEGMENT` at real 0x3000.
const CODE_RA: u64 = 0x3000;
/// Read/write data page: EA `SEGMENT + 0x1000` at real 0x5000.
const DATA_EA: u64 = SEGMENT + 0x1000;
const DATA_RA: u64 = 0x5000;
/// Read-only data page: EA `SEGMENT + 0x2000` at real 0x6000.
const RO_EA: u64 = SEGMENT + 0x2000;
/// A page of the segment no entry maps.
const HOLE_EA: u64 = SEGMENT + 0x3000;

fn lwz(rt: u32, d: u16, ra: u32) -> u32 {
    (32 << 26) | (rt << 21) | (ra << 16) | u32::from(d)
}

fn stw(rs: u32, d: u16, ra: u32) -> u32 {
    (36 << 26) | (rs << 21) | (ra << 16) | u32::from(d)
}

/// `mtmsrd rs` with L = 0.
fn mtmsrd(rs: u32) -> u32 {
    (31 << 26) | (rs << 21) | (178 << 1)
}

fn put(mem: &mut GuestMemory, at: u64, bytes: &[u8]) {
    let range = ByteRange::new(GuestAddr::new(at), bytes.len() as u64).unwrap();
    mem.apply_commit(range, bytes).unwrap();
}

/// Writes the entry for page `index` of the segment to slot 0 of its primary group.
///
/// The hash is VSID XOR the page index. Every page here has AVPN
/// VSID << 12, because the five high bits of its offset are 0.
fn map_page(mem: &mut GuestMemory, index: u64, ra: u64, pp: u64) {
    let pteg = HTAB + ((VSID ^ index) & 0x7FF) * 128;
    let d0 = (VSID << 12) | 1;
    put(mem, pteg, &d0.to_be_bytes());
    put(mem, pteg + 8, &(ra | pp).to_be_bytes());
}

/// 512 KB of memory that holds:
///
/// - the page table;
/// - `code` on the code page;
/// - 0xDEAD_BEEF at the start of the read/write page;
/// - the read-only page, mapped with PP 11.
fn memory(code: &[u32]) -> GuestMemory {
    let mut mem = GuestMemory::new(0x8_0000);
    map_page(&mut mem, 0, CODE_RA, 0b10);
    map_page(&mut mem, 1, DATA_RA, 0b10);
    map_page(&mut mem, 2, 0x6000, 0b11);
    for (i, word) in code.iter().enumerate() {
        put(&mut mem, CODE_RA + 4 * i as u64, &word.to_be_bytes());
    }
    put(&mut mem, DATA_RA, &0xDEAD_BEEFu32.to_be_bytes());
    mem
}

/// A unit at the segment's first word, with:
///
/// - MSR `msr`;
/// - SLB entry 0 over the segment;
/// - the page table at [`HTAB`].
fn kernel_unit(msr: u64) -> PpuExecutionUnit {
    let mut sup = SupervisorState::new(msr);
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

/// A read intent's start and length.
type Read = (u64, u64);
/// A write intent's start and bytes.
type Write = (u64, Vec<u8>);

/// Every read intent and write intent, in order.
fn ranges(effects: &[Effect]) -> (Vec<Read>, Vec<Write>) {
    let mut reads = Vec::new();
    let mut writes = Vec::new();
    for e in effects {
        match e {
            Effect::SharedReadIntent { range, .. } => {
                reads.push((range.start().raw(), range.length()))
            }
            Effect::SharedWriteIntent { range, bytes, .. } => {
                writes.push((range.start().raw(), bytes.bytes().to_vec()));
            }
            _ => {}
        }
    }
    (reads, writes)
}

/// Every write intent outside the page table, in order: the stores, less
/// the R and C writes of the table searches.
fn stores(effects: &[Effect]) -> Vec<Write> {
    let table = HTAB..HTAB + 0x4_0000;
    let mut writes = ranges(effects).1;
    writes.retain(|(at, _)| !table.contains(at));
    writes
}

fn commit(mem: &mut GuestMemory, effects: &[Effect]) {
    for (at, bytes) in ranges(effects).1 {
        put(mem, at, &bytes);
    }
}

#[test]
fn a_mapped_load_and_store_reach_their_pages_and_publish_real_addresses() {
    let mem = memory(&[lwz(4, 0, 5), stw(4, 4, 5)]);
    let mut unit = kernel_unit(MSR_SF | MSR_IR | MSR_DR);
    unit.state_mut().set_gpr(5, DATA_EA);
    let (r, effects) = run(&mut unit, &mem, 2);
    assert_eq!(
        r.yield_reason,
        YieldReason::BudgetExhausted,
        "{:?}",
        r.local_diagnostics
    );
    assert_eq!(unit.state().gpr[4], 0xDEAD_BEEF);
    assert_eq!(unit.state().pc, SEGMENT + 8);
    let (reads, writes) = ranges(&effects);
    assert_eq!(
        stores(&effects),
        vec![(DATA_RA + 4, 0xDEAD_BEEFu32.to_be_bytes().to_vec())]
    );
    assert!(
        reads.contains(&(DATA_RA, 4)),
        "the load's read intent: {reads:x?}"
    );
    assert!(
        reads.contains(&(CODE_RA, 8)),
        "the fetched text: {reads:x?}"
    );
    let mut published = reads.iter().map(|r| r.0).chain(writes.iter().map(|w| w.0));
    assert!(
        published.all(|a| a < 0x8_0000),
        "every published address is real: {reads:x?} {writes:x?}"
    );
}

/// [PPC-Book3 p:67 s:5.5.3] A data access with no page table entry sets DSISR bit 1, and DAR holds its EA.
#[test]
fn a_load_from_a_page_no_entry_maps_takes_the_data_storage_interrupt() {
    let mem = memory(&[lwz(4, 0, 5)]);
    let mut unit = kernel_unit(MSR_SF | MSR_IR | MSR_DR);
    unit.state_mut().set_gpr(5, HOLE_EA);
    let (r, _) = run(&mut unit, &mem, 1);
    assert_eq!(r.yield_reason, YieldReason::InterruptBoundary);
    assert_eq!(
        r.local_diagnostics.interrupt,
        Some(InterruptRecord {
            vector: irq::VECTOR_DATA_STORAGE,
            srr0: SEGMENT,
            srr1: MSR_SF | MSR_IR | MSR_DR,
            dsisr: Some(irq::DSISR_NOT_FOUND),
            dar: Some(HOLE_EA),
        })
    );
}

#[test]
fn a_load_from_a_segment_no_slb_entry_maps_takes_the_data_segment_interrupt() {
    let mem = memory(&[lwz(4, 0, 5)]);
    let mut unit = kernel_unit(MSR_SF | MSR_IR | MSR_DR);
    unit.state_mut().set_gpr(5, 0xD000_0000);
    let (r, _) = run(&mut unit, &mem, 1);
    let record = r.local_diagnostics.interrupt.expect("an interrupt");
    assert_eq!(
        (record.vector, record.dar),
        (irq::VECTOR_DATA_SEGMENT, Some(0xD000_0000))
    );
}

/// [PPC-Book3 p:45 s:4.10.1] With key 0, PP 11 is read only: a store takes the data storage interrupt with DSISR bits 4 and 6.
/// [PPC-Book3 p:67 s:5.5.3] A data storage interrupt sets SRR1 bits 33:36 and 42:47 to 0 and loads the other bits from the MSR.
#[test]
fn a_store_to_a_read_only_page_takes_the_protection_data_storage_interrupt() {
    let mem = memory(&[lwz(4, 0, 5), stw(4, 0, 5)]);
    let mut unit = kernel_unit(MSR_SF | MSR_IR | MSR_DR);
    unit.state_mut().set_gpr(5, RO_EA);
    // The load reads the page; the store faults in the next batch.
    let (r, _) = run(&mut unit, &mem, 1);
    assert_eq!(
        r.yield_reason,
        YieldReason::BudgetExhausted,
        "{:?}",
        r.local_diagnostics
    );
    let (r, _) = run(&mut unit, &mem, 1);
    assert_eq!(
        r.local_diagnostics.interrupt,
        Some(InterruptRecord {
            vector: irq::VECTOR_DATA_STORAGE,
            srr0: SEGMENT + 4,
            srr1: MSR_SF | MSR_IR | MSR_DR,
            dsisr: Some(irq::DSISR_PROTECTION | irq::DSISR_STORE),
            dar: Some(RO_EA),
        })
    );
}

/// [PPC-Book3 p:68 s:5.5.5] A fetch with no page table entry sets SRR1 bit 33.
#[test]
fn a_fetch_from_a_page_no_entry_maps_takes_the_instruction_storage_interrupt() {
    let mem = memory(&[]);
    let mut unit = kernel_unit(MSR_SF | MSR_IR | MSR_DR);
    unit.state_mut().pc = HOLE_EA;
    let (r, _) = run(&mut unit, &mem, 1);
    assert_eq!(
        r.local_diagnostics.interrupt,
        Some(InterruptRecord {
            vector: irq::VECTOR_INSTRUCTION_STORAGE,
            srr0: HOLE_EA,
            srr1: MSR_SF | MSR_IR | MSR_DR | irq::SRR1_FETCH_NOT_FOUND,
            dsisr: None,
            dar: None,
        })
    );
}

/// [PPC-Book3 p:46 s:4.10.2] Real mode refuses an access at or past the RMLR limit; [PPC-Book3 p:68 s:5.5.5] a fetch storage protection refuses sets SRR1 bit 36.
#[test]
fn a_fetch_past_the_real_mode_limit_takes_the_instruction_storage_interrupt_with_bit_36() {
    let mem = memory(&[]);
    let mut unit = kernel_unit(MSR_SF);
    unit.state_mut()
        .update_supervisor(|s| {
            s.partition = Partition::new(HTAB, 1 << 16, 0, Partition::INITIAL.lb()).unwrap();
        })
        .unwrap();
    unit.state_mut().pc = 0x8000_0000_0001_0000;
    let (r, _) = run(&mut unit, &mem, 1);
    assert_eq!(
        r.local_diagnostics.interrupt,
        Some(InterruptRecord {
            vector: irq::VECTOR_INSTRUCTION_STORAGE,
            srr0: 0x8000_0000_0001_0000,
            srr1: MSR_SF | irq::SRR1_FETCH_PROTECTION,
            dsisr: None,
            dar: None,
        })
    );
}

#[test]
fn real_mode_runs_the_kernels_high_addresses_on_the_real_bytes_below() {
    // At EA 0x8000_0000_0000_3000: lwz r4, 0x2000(r5) with r5 the same
    // high base, so the load reads real 0x5000.
    let mem = memory(&[lwz(4, 0x2000, 5)]);
    let mut unit = kernel_unit(MSR_SF);
    unit.state_mut().pc = 0x8000_0000_0000_3000;
    unit.state_mut().set_gpr(5, 0x8000_0000_0000_3000);
    let (r, effects) = run(&mut unit, &mem, 1);
    assert_eq!(
        r.yield_reason,
        YieldReason::BudgetExhausted,
        "{:?}",
        r.local_diagnostics
    );
    assert_eq!(unit.state().gpr[4], 0xDEAD_BEEF);
    let (reads, _) = ranges(&effects);
    assert!(reads.contains(&(DATA_RA, 4)), "{reads:x?}");
    assert!(reads.contains(&(CODE_RA, 4)), "{reads:x?}");
}

#[test]
fn a_load_through_a_second_alias_of_a_buffered_store_waits_for_the_store_to_commit() {
    // stw r4, 0x100(r5) through the high alias, then lwz r6, 0x100(0)
    // through the low one: the same real word.
    let mut mem = memory(&[stw(4, 0x100, 5), lwz(6, 0x100, 0)]);
    let mut unit = kernel_unit(MSR_SF);
    unit.state_mut().pc = 0x8000_0000_0000_3000;
    unit.state_mut().set_gpr(4, 0x1234_5678);
    unit.state_mut().set_gpr(5, 0x8000_0000_0000_0000);
    let (r, effects) = run(&mut unit, &mem, 8);
    assert_eq!(r.yield_reason, YieldReason::BudgetExhausted);
    assert_eq!(
        unit.retired_instructions(),
        1,
        "the batch ends before the load"
    );
    assert_eq!(
        ranges(&effects).1,
        vec![(0x100, 0x1234_5678u32.to_be_bytes().to_vec())]
    );
    commit(&mut mem, &effects);
    let (r, _) = run(&mut unit, &mem, 1);
    assert_eq!(r.yield_reason, YieldReason::BudgetExhausted);
    assert_eq!(unit.state().gpr[6], 0x1234_5678);
}

#[test]
fn a_move_that_turns_data_relocation_off_ends_the_batch_and_the_next_load_is_real() {
    let mem = memory(&[mtmsrd(7), lwz(4, 0, 5)]);
    let mut unit = kernel_unit(MSR_SF | MSR_IR | MSR_DR);
    unit.state_mut().set_gpr(7, MSR_SF | MSR_IR);
    unit.state_mut().set_gpr(5, DATA_RA);
    let (r, _) = run(&mut unit, &mem, 8);
    assert_eq!(
        r.yield_reason,
        YieldReason::BudgetExhausted,
        "{:?}",
        r.local_diagnostics
    );
    assert_eq!(
        unit.retired_instructions(),
        1,
        "the batch ends at the MSR move"
    );
    // With DR = 0, EA 0x5000 is real 0x5000; with DR = 1 its segment has
    // no SLB entry.
    let (r, effects) = run(&mut unit, &mem, 1);
    assert_eq!(
        r.yield_reason,
        YieldReason::BudgetExhausted,
        "{:?}",
        r.local_diagnostics
    );
    assert_eq!(unit.state().gpr[4], 0xDEAD_BEEF);
    assert!(ranges(&effects).0.contains(&(DATA_RA, 4)));
}

#[test]
fn a_page_table_change_reaches_the_thread_at_its_next_batch() {
    let mut mem = memory(&[lwz(4, 0, 5)]);
    put(&mut mem, 0x7000, &0x1111_1111u32.to_be_bytes());
    let mut unit = kernel_unit(MSR_SF | MSR_IR | MSR_DR);
    unit.state_mut().set_gpr(5, DATA_EA);
    run(&mut unit, &mem, 1);
    assert_eq!(unit.state().gpr[4], 0xDEAD_BEEF);
    // Move the data page to real 0x7000.
    map_page(&mut mem, 1, 0x7000, 0b10);
    unit.state_mut().pc = SEGMENT;
    run(&mut unit, &mem, 1);
    assert_eq!(unit.state().gpr[4], 0x1111_1111);
}

#[test]
fn a_restored_unit_runs_a_batch_as_the_unit_it_was_snapshotted_from() {
    // A load through the low alias of a real word, then a store through
    // the high alias and two adds. A translation of the low alias kept
    // from the first batch would end the second at the store.
    let addi_r8 = (14 << 26) | (8 << 21) | (8 << 16) | 1;
    let mem = memory(&[lwz(6, 0x100, 0), stw(4, 0x100, 5), addi_r8, addi_r8]);
    let mut unit = kernel_unit(MSR_SF);
    unit.state_mut().pc = 0x8000_0000_0000_3000;
    unit.state_mut().set_gpr(4, 0x1234_5678);
    unit.state_mut().set_gpr(5, 0x8000_0000_0000_0000);
    run(&mut unit, &mem, 1);
    let snapshot = unit.snapshot();
    let mut restored = PpuExecutionUnit::new(UnitId::new(0));
    restored.restore(snapshot);
    let (a, a_effects) = run(&mut unit, &mem, 3);
    let (b, b_effects) = run(&mut restored, &mem, 3);
    assert_eq!(
        unit.retired_instructions(),
        4,
        "the batch runs past the store"
    );
    assert_eq!(restored.retired_instructions(), 3);
    assert_eq!(a.yield_reason, b.yield_reason);
    assert_eq!(a_effects, b_effects);
    assert_eq!(unit.snapshot(), restored.snapshot());
}

/// `lwarx rt,0,rb`.
fn lwarx(rt: u32, rb: u32) -> u32 {
    (31 << 26) | (rt << 21) | (rb << 11) | (20 << 1)
}

/// `stwcx. rs,0,rb`.
fn stwcx(rs: u32, rb: u32) -> u32 {
    (31 << 26) | (rs << 21) | (rb << 11) | (150 << 1) | 1
}

#[test]
fn a_conditional_store_with_a_load_after_it_publishes_every_address_real() {
    // The flush puts the conditional store ahead of the later load's read
    // intent, which is real already.
    let mem = memory(&[lwarx(4, 5), stwcx(4, 5), lwz(6, 4, 5)]);
    let mut unit = kernel_unit(MSR_SF | MSR_IR | MSR_DR);
    unit.state_mut().set_gpr(5, DATA_EA);
    let (r, effects) = run(&mut unit, &mem, 3);
    assert_eq!(
        r.yield_reason,
        YieldReason::BudgetExhausted,
        "{:?}",
        r.local_diagnostics
    );
    assert_eq!(unit.retired_instructions(), 3);
    let conditional: Vec<(u64, u64)> = effects
        .iter()
        .filter_map(|e| match e {
            Effect::ConditionalStore { range, .. } => Some((range.start().raw(), range.length())),
            _ => None,
        })
        .collect();
    assert_eq!(conditional, vec![(DATA_RA, 4)]);
    let (reads, _) = ranges(&effects);
    assert!(
        reads.contains(&(DATA_RA + 4, 4)),
        "the load after the conditional store: {reads:x?}"
    );
}

#[test]
fn a_code_page_at_an_effective_address_below_its_real_address_runs() {
    // Segment 0 over the same VSID: EA 0 is the code page at real 0x3000.
    let mem = memory(&[lwz(4, 0, 5)]);
    let mut unit = kernel_unit(MSR_SF | MSR_IR | MSR_DR);
    unit.state_mut()
        .update_supervisor(|s| {
            s.slb[1] = SlbEntry {
                esid_v: SLB_VALID,
                vsid_flags: VSID << 12,
            };
        })
        .unwrap();
    unit.state_mut().pc = 0;
    unit.state_mut().set_gpr(5, DATA_EA);
    let (r, effects) = run(&mut unit, &mem, 1);
    assert_eq!(
        r.yield_reason,
        YieldReason::BudgetExhausted,
        "{:?}",
        r.local_diagnostics
    );
    assert_eq!(unit.state().gpr[4], 0xDEAD_BEEF);
    assert!(ranges(&effects).0.contains(&(CODE_RA, 4)));
}

#[test]
fn a_store_through_one_cached_alias_ends_the_batch_before_a_load_through_the_other() {
    // lwz r6, 0x100(0) caches the low alias, stw r4, 0x100(r5) writes the
    // same real word through the high one, and lwz r7, 0x100(0) reads it
    // back through the low one.
    let mut mem = memory(&[lwz(6, 0x100, 0), stw(4, 0x100, 5), lwz(7, 0x100, 0)]);
    let mut unit = kernel_unit(MSR_SF);
    unit.state_mut().pc = 0x8000_0000_0000_3000;
    unit.state_mut().set_gpr(4, 0x1234_5678);
    unit.state_mut().set_gpr(5, 0x8000_0000_0000_0000);
    let (r, effects) = run(&mut unit, &mem, 3);
    assert_eq!(
        r.yield_reason,
        YieldReason::BudgetExhausted,
        "{:?}",
        r.local_diagnostics
    );
    assert_eq!(
        unit.retired_instructions(),
        2,
        "the batch ends at the store"
    );
    commit(&mut mem, &effects);
    let (r, _) = run(&mut unit, &mem, 1);
    assert_eq!(
        r.yield_reason,
        YieldReason::BudgetExhausted,
        "{:?}",
        r.local_diagnostics
    );
    assert_eq!(unit.state().gpr[7], 0x1234_5678);
}

/// The last page of the effective space.
const TOP_PAGE: u64 = 0xFFFF_FFFF_FFFF_F000;
/// The real page [`TOP_PAGE`] maps to.
const TOP_RA: u64 = 0x7000;

/// Maps [`TOP_PAGE`] read/write at [`TOP_RA`] through SLB entry 2 over the
/// last segment, with the same VSID.
fn map_top_page(unit: &mut PpuExecutionUnit, mem: &mut GuestMemory) {
    unit.state_mut()
        .update_supervisor(|s| {
            s.slb[2] = SlbEntry {
                esid_v: (TOP_PAGE & !0x0FFF_FFFF) | SLB_VALID,
                vsid_flags: VSID << 12,
            };
        })
        .unwrap();
    // Page 0xFFFF of the segment: the five high bits of its offset are 1,
    // and the AVPN holds them below the VSID.
    let pteg = HTAB + ((VSID ^ 0xFFFF) & 0x7FF) * 128;
    let d0 = (VSID << 12) | (0x1F << 7) | 1;
    put(mem, pteg, &d0.to_be_bytes());
    put(mem, pteg + 8, &(TOP_RA | 0b10).to_be_bytes());
}

#[test]
fn a_load_from_the_last_page_of_the_effective_space_reads_its_real_page() {
    let mut mem = memory(&[lwz(4, 0, 5)]);
    let mut unit = kernel_unit(MSR_SF | MSR_IR | MSR_DR);
    map_top_page(&mut unit, &mut mem);
    put(&mut mem, TOP_RA, &0x1111_1111u32.to_be_bytes());
    unit.state_mut().set_gpr(5, TOP_PAGE);
    let (r, effects) = run(&mut unit, &mem, 1);
    assert_eq!(
        r.yield_reason,
        YieldReason::BudgetExhausted,
        "{:?}",
        r.local_diagnostics
    );
    assert_eq!(unit.state().gpr[4], 0x1111_1111);
    assert!(ranges(&effects).0.contains(&(TOP_RA, 4)));
}

/// [PPC-Book1 p:15 s:1.12.2] In 64-bit mode the effective address arithmetic wraps from 2^64 - 1 to 0.
/// [PPC-Book3 p:68 s:5.5.4] A store with a byte whose EA has no segment takes the data segment interrupt, with DAR at a byte of the first doubleword accessed in that segment.
#[test]
fn a_store_that_wraps_into_a_segment_no_slb_entry_maps_takes_the_data_segment_interrupt_and_stores_nothing(
) {
    // stw r4, 0xFFE(r5) with r5 the last page: the word's last two bytes
    // wrap to EA 0, whose segment no SLB entry maps.
    let mut mem = memory(&[stw(4, 0xFFE, 5)]);
    let mut unit = kernel_unit(MSR_SF | MSR_IR | MSR_DR);
    map_top_page(&mut unit, &mut mem);
    unit.state_mut().set_gpr(5, TOP_PAGE);
    let (r, effects) = run(&mut unit, &mem, 1);
    assert_eq!(r.yield_reason, YieldReason::InterruptBoundary);
    let record = r.local_diagnostics.interrupt.expect("an interrupt");
    assert_eq!(
        (record.vector, record.srr0),
        (irq::VECTOR_DATA_SEGMENT, SEGMENT)
    );
    assert!(
        record.dar.is_some_and(|dar| dar < 8),
        "DAR {:x?}",
        record.dar
    );
    assert_eq!(
        stores(&effects),
        Vec::<Write>::new(),
        "the store does not publish"
    );
}

/// `lbz rt,d(ra)`.
fn lbz(rt: u32, d: u16, ra: u32) -> u32 {
    (34 << 26) | (rt << 21) | (ra << 16) | u32::from(d)
}

/// [PPC-Book1 p:15 s:1.12.2] In 64-bit mode the whole 64-bit result is the effective address, up to the maximum address 2^64 - 1.
#[test]
fn a_load_of_the_last_byte_of_the_effective_space_reads_its_real_byte() {
    // lbz r4, 0xFFF(r5) with r5 the last page: EA 2^64 - 1.
    let mut mem = memory(&[lbz(4, 0xFFF, 5)]);
    let mut unit = kernel_unit(MSR_SF | MSR_IR | MSR_DR | MSR_ME);
    map_top_page(&mut unit, &mut mem);
    put(&mut mem, TOP_RA + 0xFFF, &[0xC3]);
    unit.state_mut().set_gpr(5, TOP_PAGE);
    let (r, effects) = run(&mut unit, &mem, 1);
    assert_eq!(
        r.yield_reason,
        YieldReason::BudgetExhausted,
        "{:?}",
        r.local_diagnostics
    );
    assert_eq!(unit.state().gpr[4], 0xC3);
    assert!(ranges(&effects).0.contains(&(TOP_RA + 0xFFF, 1)));
}

/// `isync`.
const ISYNC: u32 = 0x4C00_012C;
/// `ori r0,r0,0`.
const NOP: u32 = 0x6000_0000;

/// `ba -4`: a branch to the last word of the effective space.
const BA_LAST_WORD: u32 = 0x4BFF_FFFE;

#[test]
fn a_branch_to_the_last_word_of_the_effective_space_stops_as_unmodelled() {
    // The word's next address wraps to 0, which CellGov does not model.
    let mut mem = memory(&[BA_LAST_WORD]);
    let mut unit = kernel_unit(MSR_SF | MSR_IR | MSR_DR);
    map_top_page(&mut unit, &mut mem);
    put(&mut mem, TOP_RA + 0xFFC, &NOP.to_be_bytes());
    let (mut r, _) = run(&mut unit, &mem, 3);
    // The batch may end at the branch; the next one fetches its target.
    for _ in 0..1 {
        if r.fault.is_some() {
            break;
        }
        r = run(&mut unit, &mem, 3).0;
    }
    assert_eq!(stop(&r), (Some(FAULT_UNMODELLED), None));
    assert_eq!(unit.retired_instructions(), 1, "the branch retires");
    assert_eq!(unit.state().pc, u64::MAX - 3);
}

#[test]
fn the_fetch_window_of_the_last_page_ends_at_the_word_before_the_last() {
    let mut mem = memory(&[]);
    let mut unit = kernel_unit(MSR_SF | MSR_IR | MSR_DR);
    map_top_page(&mut unit, &mut mem);
    let ((start, last), _) = unit
        .translated_fetch(&mem, TOP_PAGE)
        .expect("the last page fetches");
    assert_eq!((start, last), (TOP_PAGE, u64::MAX - 7));
    let ((_, last), _) = unit
        .translated_fetch(&mem, u64::MAX - 7)
        .expect("the word before the last fetches");
    assert_eq!(last, u64::MAX - 7);
}

#[test]
fn an_instruction_stored_through_one_alias_runs_after_isync_through_the_other() {
    // At the high alias of the code page: stw r4, 8(r5) with r5 the low
    // alias writes `li r7, 0x55` over the nop in slot 2, then isync.
    let li_r7 = (14 << 26) | (7 << 21) | 0x55;
    let mut mem = memory(&[stw(4, 8, 5), ISYNC, NOP]);
    let mut unit = kernel_unit(MSR_SF);
    unit.state_mut().pc = 0x8000_0000_0000_3000;
    unit.state_mut().set_gpr(4, li_r7);
    unit.state_mut().set_gpr(5, CODE_RA);
    let (r, effects) = run(&mut unit, &mem, 3);
    assert_eq!(
        r.yield_reason,
        YieldReason::BudgetExhausted,
        "{:?}",
        r.local_diagnostics
    );
    assert_eq!(
        unit.retired_instructions(),
        2,
        "the batch ends before the fetch"
    );
    commit(&mut mem, &effects);
    let (r, _) = run(&mut unit, &mem, 1);
    assert_eq!(
        r.yield_reason,
        YieldReason::BudgetExhausted,
        "{:?}",
        r.local_diagnostics
    );
    assert_eq!(unit.state().gpr[7], 0x55);
}

/// A real address past the 512 KB the test memory holds.
const UNBACKED_RA: u64 = 0x9_0000;

/// The guest fault code of a run's terminal stop, and the vector of the
/// interrupt it recorded. An interrupt the thread takes at its vector has
/// no fault code.
fn stop(r: &ExecutionStepResult) -> (Option<u32>, Option<u64>) {
    let code = match r.fault {
        Some(cellgov_effects::FaultKind::Guest(code)) => Some(code),
        _ => None,
    };
    (code, r.local_diagnostics.interrupt.map(|i| i.vector))
}

/// The PTE caching-inhibited bit, for [`map_page`]'s `pp`.
const PTE_I: u64 = 0x20;

/// [PPC-Book3 p:66 s:5.5.2] An access to nonexistent storage may enter the checkstop state; [CBE-Handbook p:247 s:9.5.2] the PPE's one machine check source is a caching-inhibited load.
#[test]
fn only_a_caching_inhibited_load_with_machine_checks_enabled_takes_the_machine_check() {
    let checkstop = (Some(FAULT_CHECKSTOP), None);
    let machine_check = (Some(FAULT_MACHINE_CHECK), Some(irq::VECTOR_MACHINE_CHECK));
    // (instruction, PTE bits, MSR[ME], stop).
    let cases = [
        (lwz(4, 0, 5), 0b10 | PTE_I, MSR_ME, machine_check),
        (lwz(4, 0, 5), 0b10 | PTE_I, 0, checkstop),
        (lwz(4, 0, 5), 0b10, MSR_ME, checkstop),
        (stw(4, 0, 5), 0b10 | PTE_I, MSR_ME, checkstop),
    ];
    for (insn, pte, me, want) in cases {
        let mut mem = memory(&[insn]);
        map_page(&mut mem, 4, UNBACKED_RA, pte);
        let mut unit = kernel_unit(MSR_SF | MSR_IR | MSR_DR | me);
        unit.state_mut().set_gpr(5, SEGMENT + 0x4000);
        let (r, _) = run(&mut unit, &mem, 1);
        assert_eq!(stop(&r), want, "insn {insn:#x} pte {pte:#x} me {me:#x}");
    }
}

#[test]
fn a_real_mode_fetch_from_no_storage_checkstops() {
    let mem = memory(&[]);
    let mut unit = kernel_unit(MSR_SF | MSR_ME);
    unit.state_mut().pc = 0x8000_0000_0000_0000 | UNBACKED_RA;
    let (r, _) = run(&mut unit, &mem, 1);
    assert_eq!(stop(&r), (Some(FAULT_CHECKSTOP), None));
}

#[test]
fn a_page_table_group_no_storage_holds_checkstops_the_search() {
    let mem = memory(&[lwz(4, 0, 5)]);
    let mut unit = kernel_unit(MSR_SF | MSR_IR | MSR_DR | MSR_ME);
    // A table at 1 MB, past the 512 KB the memory holds.
    unit.state_mut()
        .update_supervisor(|s| {
            s.partition = Partition::new(0x10_0000, 1 << 62, 0, Partition::INITIAL.lb()).unwrap();
        })
        .unwrap();
    unit.state_mut().set_gpr(5, DATA_EA);
    unit.state_mut().pc = CODE_RA;
    unit.state_mut()
        .update_supervisor(|s| s.msr &= !MSR_IR)
        .unwrap();
    let (r, _) = run(&mut unit, &mem, 1);
    assert_eq!(stop(&r), (Some(FAULT_CHECKSTOP), None));
}

/// [PPC-Book2 p:10 s:1.7.3.1] A reservation is on a reservation granule of storage, so a conditional store through another effective address of the granule finds it.
#[test]
fn a_reservation_through_one_alias_is_held_for_a_conditional_store_through_the_other() {
    // r5 is the high alias of real 0x100 and r7 the low one: one real
    // granule. lwarx through each, stwcx. through the other.
    for (reserve, store) in [(5, 7), (7, 5)] {
        let mem = memory(&[lwarx(4, reserve), stwcx(6, store)]);
        let mut unit = kernel_unit(MSR_SF);
        unit.state_mut().pc = 0x8000_0000_0000_3000;
        unit.state_mut().set_gpr(5, 0x8000_0000_0000_0100);
        unit.state_mut().set_gpr(6, 0x0BAD_F00D);
        unit.state_mut().set_gpr(7, 0x100);
        let (r, effects) = run(&mut unit, &mem, 2);
        let case = format!("lwarx through r{reserve}");
        assert_eq!(
            r.yield_reason,
            YieldReason::BudgetExhausted,
            "{case}: {:?}",
            r.local_diagnostics
        );
        assert_eq!(
            unit.state().cr() >> 28 & 0b0010,
            0b0010,
            "{case}: stwcx. succeeds"
        );
        let reserved: Vec<u64> = effects
            .iter()
            .filter_map(|e| match e {
                Effect::ReservationAcquire { line_addr, .. } => Some(*line_addr),
                _ => None,
            })
            .collect();
        assert_eq!(
            reserved,
            vec![0x100],
            "{case}: the reservation names the real line"
        );
        assert!(
            effects.iter().any(|e| matches!(
                e,
                Effect::ConditionalStore { range, .. } if range.start().raw() == 0x100
            )),
            "{case}"
        );
    }
}

#[test]
fn a_supervisor_thread_in_32_bit_mode_stops_as_unmodelled_before_it_runs() {
    let mem = memory(&[lwz(4, 0, 5)]);
    let mut unit = kernel_unit(0);
    unit.state_mut().pc = CODE_RA;
    unit.state_mut().set_gpr(5, DATA_RA);
    let (r, effects) = run(&mut unit, &mem, 1);
    assert_eq!(stop(&r), (Some(FAULT_UNMODELLED), None));
    assert_eq!(unit.retired_instructions(), 0);
    assert!(effects.is_empty());
    // The same thread in 64-bit mode runs the load.
    let mut unit = kernel_unit(MSR_SF);
    unit.state_mut().pc = CODE_RA;
    unit.state_mut().set_gpr(5, DATA_RA);
    run(&mut unit, &mem, 1);
    assert_eq!(unit.state().gpr[4], 0xDEAD_BEEF);
}

#[test]
fn a_move_that_clears_sf_stops_the_thread_at_its_next_instruction() {
    let mem = memory(&[mtmsrd(7), lwz(4, 0, 5)]);
    let mut unit = kernel_unit(MSR_SF | MSR_IR | MSR_DR);
    unit.state_mut().set_gpr(7, MSR_IR | MSR_DR);
    let (r, _) = run(&mut unit, &mem, 8);
    assert_eq!(
        r.yield_reason,
        YieldReason::BudgetExhausted,
        "{:?}",
        r.local_diagnostics
    );
    assert_eq!(unit.retired_instructions(), 1);
    let (r, _) = run(&mut unit, &mem, 8);
    assert_eq!(stop(&r), (Some(FAULT_UNMODELLED), None));
    assert_eq!(unit.state().pc, SEGMENT + 4);
}

#[test]
fn a_store_across_two_pages_has_a_real_piece_in_each() {
    use crate::mmu::{Access, Translation};
    let mut unit = kernel_unit(MSR_SF | MSR_IR | MSR_DR);
    for (ea, ra) in [(0x1000, 0x5000), (0x2000, 0x7000)] {
        unit.translations.insert(
            Access::Store,
            Translation {
                ea,
                ra,
                len: 0x1000,
                writable: true,
                inhibited: false,
                changed: false,
            },
        );
    }
    let pieces: Vec<(u64, u8)> = unit.real_store_pieces(0x1FFC, 8).collect();
    assert_eq!(pieces, vec![(0x5FFC, 4), (0x7000, 4)]);
}

/// Counts every dispatch.
struct Dispatches(std::cell::RefCell<u64>);

impl crate::tap::PpuTap for Dispatches {
    fn dispatch(
        &self,
        _: UnitId,
        _: &crate::instruction::PpuInstruction,
        _: &crate::state::PpuState,
    ) {
        *self.0.borrow_mut() += 1;
    }
}

#[test]
fn an_instruction_that_runs_again_after_a_translation_miss_dispatches_once() {
    let mem = memory(&[lwz(4, 0, 5), stw(4, 4, 5)]);
    let mut unit = kernel_unit(MSR_SF | MSR_IR | MSR_DR);
    unit.state_mut().set_gpr(5, DATA_EA);
    let tap = std::rc::Rc::new(Dispatches(std::cell::RefCell::new(0)));
    unit.set_tap(tap.clone());
    let (r, _) = run(&mut unit, &mem, 2);
    assert_eq!(
        r.yield_reason,
        YieldReason::BudgetExhausted,
        "{:?}",
        r.local_diagnostics
    );
    assert_eq!(unit.retired_instructions(), 2);
    assert_eq!(
        *tap.0.borrow(),
        2,
        "the load's miss reran it without a second dispatch"
    );
}

/// The real page that page 2 of the segment maps to in the straddle tests.
///
/// It is not the page after [`DATA_RA`], so one real range cannot hold
/// both pieces.
const FAR_RA: u64 = 0x7000;

#[test]
fn a_store_across_two_pages_publishes_each_real_piece_with_its_own_bytes() {
    // stw r4, 0xFFE(r5) with r5 at page 1: two bytes at the end of real
    // 0x5000, two at the start of real 0x7000.
    let mut mem = memory(&[stw(4, 0xFFE, 5)]);
    map_page(&mut mem, 2, FAR_RA, 0b10);
    let mut unit = kernel_unit(MSR_SF | MSR_IR | MSR_DR);
    unit.state_mut().set_gpr(4, 0x1234_5678);
    unit.state_mut().set_gpr(5, DATA_EA);
    let (r, effects) = run(&mut unit, &mem, 1);
    assert_eq!(
        r.yield_reason,
        YieldReason::BudgetExhausted,
        "{:?}",
        r.local_diagnostics
    );
    assert_eq!(
        stores(&effects),
        vec![
            (DATA_RA + 0xFFE, vec![0x12, 0x34]),
            (FAR_RA, vec![0x56, 0x78]),
        ]
    );
}

#[test]
fn a_load_across_two_pages_reads_both_real_pages_and_publishes_a_read_of_each() {
    let mut mem = memory(&[lwz(4, 0xFFE, 5)]);
    map_page(&mut mem, 2, FAR_RA, 0b10);
    put(&mut mem, DATA_RA + 0xFFE, &[0xAA, 0xBB]);
    put(&mut mem, FAR_RA, &[0xCC, 0xDD]);
    let mut unit = kernel_unit(MSR_SF | MSR_IR | MSR_DR);
    unit.state_mut().set_gpr(5, DATA_EA);
    let (r, effects) = run(&mut unit, &mem, 1);
    assert_eq!(
        r.yield_reason,
        YieldReason::BudgetExhausted,
        "{:?}",
        r.local_diagnostics
    );
    assert_eq!(unit.state().gpr[4], 0xAABB_CCDD);
    let (reads, _) = ranges(&effects);
    assert_eq!(
        reads,
        vec![
            (DATA_RA + 0xFFE, 2),
            (FAR_RA, 2),
            (CODE_RA, 4),
            group_read(0),
            group_read(1),
            group_read(2)
        ],
        "the load's two real pieces, the fetched text, then the groups the misses read"
    );
}

#[test]
fn a_fetch_run_across_two_pages_publishes_a_real_read_of_each_page() {
    // Two nops at the end of the code page run on into two at the start
    // of page 1, at real 0x5000.
    let mut mem = memory(&[]);
    for at in [CODE_RA + 0xFF8, CODE_RA + 0xFFC, DATA_RA, DATA_RA + 4] {
        put(&mut mem, at, &NOP.to_be_bytes());
    }
    let mut unit = kernel_unit(MSR_SF | MSR_IR | MSR_DR);
    unit.state_mut().pc = SEGMENT + 0xFF8;
    let (r, effects) = run(&mut unit, &mem, 4);
    assert_eq!(
        r.yield_reason,
        YieldReason::BudgetExhausted,
        "{:?}",
        r.local_diagnostics
    );
    assert_eq!(unit.retired_instructions(), 4);
    assert_eq!(
        ranges(&effects).0,
        vec![
            (CODE_RA + 0xFF8, 8),
            (DATA_RA, 8),
            group_read(0),
            group_read(1)
        ]
    );
}

/// The read intent of the primary group of page `index` of the segment.
fn group_read(index: u64) -> Read {
    (HTAB + ((VSID ^ index) & 0x7FF) * 128, 128)
}

#[test]
fn a_translation_miss_publishes_a_read_of_each_page_table_group_it_searched() {
    // The fetch misses on page 0 and the load on page 1; each finds its
    // entry in slot 0 of its primary group.
    let mem = memory(&[lwz(4, 0, 5)]);
    let mut unit = kernel_unit(MSR_SF | MSR_IR | MSR_DR);
    unit.state_mut().set_gpr(5, DATA_EA);
    let (r, effects) = run(&mut unit, &mem, 1);
    assert_eq!(
        r.yield_reason,
        YieldReason::BudgetExhausted,
        "{:?}",
        r.local_diagnostics
    );
    assert_eq!(
        ranges(&effects).0,
        vec![(DATA_RA, 4), (CODE_RA, 4), group_read(0), group_read(1)],
        "the load, the fetched text, then the groups the two misses read"
    );
}

#[test]
fn a_miss_that_finds_no_entry_in_the_primary_group_reads_the_secondary_group_too() {
    // Page 1's entry moves to slot 0 of its secondary group, whose hash
    // is the primary hash complemented within the table.
    let mut mem = memory(&[lwz(4, 0, 5)]);
    let primary = group_read(1).0;
    put(&mut mem, primary, &[0; 16]);
    let secondary = HTAB + (!(VSID ^ 1) & 0x7FF) * 128;
    let d0 = (VSID << 12) | 0b11;
    put(&mut mem, secondary, &d0.to_be_bytes());
    put(&mut mem, secondary + 8, &(DATA_RA | 0b10).to_be_bytes());
    let mut unit = kernel_unit(MSR_SF | MSR_IR | MSR_DR);
    unit.state_mut().set_gpr(5, DATA_EA);
    let (r, effects) = run(&mut unit, &mem, 1);
    assert_eq!(
        r.yield_reason,
        YieldReason::BudgetExhausted,
        "{:?}",
        r.local_diagnostics
    );
    assert_eq!(unit.state().gpr[4], 0xDEAD_BEEF);
    let reads = ranges(&effects).0;
    assert_eq!(
        reads[reads.len() - 2..],
        [(primary, 128), (secondary, 128)],
        "{reads:x?}"
    );
}

/// [PPC-Book3 p:62 s:5.4] An interrupt sets MSR bits IR and DR to 0 and MSR bit SF to 1.
#[test]
fn a_batch_that_ends_at_a_fault_leaves_no_page_table_read_to_the_next() {
    // The fetch miss reads page 0's group and the load takes the data
    // storage interrupt. The thread then runs a nop at the vector, real
    // 0x300, in real mode.
    let mut mem = memory(&[lwz(4, 0, 5)]);
    put(&mut mem, 0x300, &NOP.to_be_bytes());
    let mut unit = kernel_unit(MSR_SF | MSR_IR | MSR_DR);
    unit.state_mut().set_gpr(5, HOLE_EA);
    let (r, _) = run(&mut unit, &mem, 1);
    assert_eq!(r.yield_reason, YieldReason::InterruptBoundary);
    assert_eq!(unit.state().pc, 0x300);
    assert_eq!(unit.state().supervisor().map(|s| s.msr), Some(MSR_SF));
    let (r, effects) = run(&mut unit, &mem, 1);
    assert_eq!(
        r.yield_reason,
        YieldReason::BudgetExhausted,
        "{:?}",
        r.local_diagnostics
    );
    assert_eq!(ranges(&effects).0, vec![(0x300, 4)]);
}

#[test]
fn instructions_in_the_last_fetch_block_of_the_effective_space_retire() {
    // Three nops from 2^64 - 16 to the word before the last: the fetch
    // block they open ends at 2^64.
    let mut mem = memory(&[]);
    let mut unit = kernel_unit(MSR_SF | MSR_IR | MSR_DR);
    map_top_page(&mut unit, &mut mem);
    for at in [TOP_RA + 0xFF0, TOP_RA + 0xFF4, TOP_RA + 0xFF8] {
        put(&mut mem, at, &NOP.to_be_bytes());
    }
    unit.state_mut().pc = TOP_PAGE + 0xFF0;
    let (r, _) = run(&mut unit, &mem, 3);
    assert_eq!(
        r.yield_reason,
        YieldReason::BudgetExhausted,
        "{:?}",
        r.local_diagnostics
    );
    assert_eq!(unit.retired_instructions(), 3);
    assert_eq!(unit.state().pc, u64::MAX - 3);
}

/// `dcbst ra,rb`.
fn dcbst(ra: u32, rb: u32) -> u32 {
    (31 << 26) | (ra << 16) | (rb << 11) | (54 << 1)
}

/// `icbi ra,rb`.
fn icbi(ra: u32, rb: u32) -> u32 {
    (31 << 26) | (ra << 16) | (rb << 11) | (982 << 1)
}

#[test]
fn a_translating_threads_code_events_carry_real_addresses() {
    // stw r4, 0xFFE(r5) across pages 1 and 2, then dcbst and icbi of
    // page 1's first block.
    use cellgov_exec::CodeEvent;
    let mut mem = memory(&[stw(4, 0xFFE, 5), dcbst(0, 5), icbi(0, 5)]);
    map_page(&mut mem, 2, FAR_RA, 0b10);
    let mut unit = kernel_unit(MSR_SF | MSR_IR | MSR_DR);
    unit.state_mut().set_gpr(4, 0x1234_5678);
    unit.state_mut().set_gpr(5, DATA_EA);
    let ctx = ExecutionContext::new(&mem).with_trace_per_step(true);
    let mut events = Vec::new();
    for _ in 0..3 {
        let mut effects = Vec::new();
        unit.run_until_yield(Budget::new(3), &ctx, &mut effects);
        events.extend(unit.drain_code_events());
        if unit.retired_instructions() == 3 {
            break;
        }
    }
    assert_eq!(unit.retired_instructions(), 3);
    let store = |addr, value| CodeEvent::Store {
        addr,
        len: 2,
        value,
        atomic: false,
    };
    let fetch = |pc| CodeEvent::Fetch { pc };
    assert_eq!(
        events,
        vec![
            fetch(CODE_RA),
            store(DATA_RA + 0xFFE, 0x1234),
            store(FAR_RA, 0x5678),
            fetch(CODE_RA + 4),
            CodeEvent::Dcbst { addr: DATA_RA },
            fetch(CODE_RA + 8),
            CodeEvent::Icbi { addr: DATA_RA },
        ]
    );
}

/// `std rs,ds(ra)`.
fn std(rs: u32, ds: u16, ra: u32) -> u32 {
    (62 << 26) | (rs << 21) | (ra << 16) | u32::from(ds & !3)
}

/// `ptesync`.
const PTESYNC: u32 = 0x7C40_04AC;

/// [PPC-Book3 p:59 s:4.12] `ptesync` orders a page table update before the next page table search.
#[test]
fn a_search_after_ptesync_sees_the_page_table_entry_the_thread_stored() {
    // Page 3 maps to DATA_RA. Page 5 maps the page that holds page 3's
    // group, and the thread stores a new doubleword 1 there that moves
    // page 3 to FAR_RA, runs ptesync, then loads from page 3.
    let group = HTAB + ((VSID ^ 3) & 0x7FF) * 128;
    let mut mem = memory(&[std(8, 8, 7), PTESYNC, lwz(4, 0, 5)]);
    map_page(&mut mem, 3, DATA_RA, 0b10);
    map_page(&mut mem, 5, group & !0xFFF, 0b10);
    put(&mut mem, FAR_RA, &0x1111_1111u32.to_be_bytes());
    let mut unit = kernel_unit(MSR_SF | MSR_IR | MSR_DR);
    unit.state_mut().set_gpr(5, HOLE_EA);
    unit.state_mut()
        .set_gpr(7, SEGMENT + 0x5000 + (group & 0xFFF));
    unit.state_mut().set_gpr(8, FAR_RA | 0b10);
    for _ in 0..3 {
        let (r, effects) = run(&mut unit, &mem, 3);
        assert_eq!(r.fault, None, "{:?}", r.local_diagnostics);
        commit(&mut mem, &effects);
        if unit.retired_instructions() == 3 {
            break;
        }
    }
    assert_eq!(unit.retired_instructions(), 3);
    assert_eq!(
        unit.state().gpr[4],
        0x1111_1111,
        "the load reads the new page"
    );
}

/// [CBE-Handbook p:135 s:6.1] The L1 data cache has real-addressed tags, so two effective aliases of one real line are one line.
#[test]
fn a_load_through_a_second_alias_of_a_line_costs_what_a_load_through_the_first_costs() {
    // lwz r4 from page 1 at DATA_RA, then lwz r6 through r7: page 6, a
    // second alias of DATA_RA, or page 1 again. The addi waits for r6.
    let addi_r8_r6 = (14 << 26) | (8 << 21) | (6 << 16) | 1;
    let cost = |second_ea: u64| {
        let mut mem = memory(&[lwz(4, 0, 5), lwz(6, 0, 7), addi_r8_r6]);
        map_page(&mut mem, 6, DATA_RA, 0b10);
        let mut unit = kernel_unit(MSR_SF | MSR_IR | MSR_DR);
        unit.state_mut().set_gpr(5, DATA_EA);
        unit.state_mut().set_gpr(7, second_ea);
        let (r, _) = run(&mut unit, &mem, 3);
        assert_eq!(
            r.yield_reason,
            YieldReason::BudgetExhausted,
            "{:?}",
            r.local_diagnostics
        );
        assert_eq!(unit.state().gpr[6], 0xDEAD_BEEF);
        r.consumed_cost.raw()
    };
    assert_eq!(cost(SEGMENT + 0x6000), cost(DATA_EA));
}

/// `sth rs,d(ra)`.
fn sth(rs: u32, d: u16, ra: u32) -> u32 {
    (44 << 26) | (rs << 21) | (ra << 16) | u32::from(d)
}

/// `dcbz ra,rb`.
fn dcbz(ra: u32, rb: u32) -> u32 {
    (31 << 26) | (ra << 16) | (rb << 11) | (1014 << 1)
}

/// The PTE change bit, for [`map_page`]'s `pp`.
const PTE_C: u64 = 0x80;

/// Three parts of how a run stopped:
///
/// - the class of its terminal stop, or `None` for an interrupt the
///   thread takes at its vector;
/// - the vector of the interrupt it recorded;
/// - whether that interrupt names a protection cause.
fn stop_class(r: &ExecutionStepResult) -> (Option<u32>, Option<u64>, bool) {
    let (code, vector) = stop(r);
    let protection = r
        .local_diagnostics
        .interrupt
        .and_then(|i| i.dsisr)
        .is_some_and(|d| d & irq::DSISR_PROTECTION != 0);
    (code.map(|c| c & 0xFFFF_0000), vector, protection)
}

/// [CBE-Handbook p:290 s:9.10] A store's alignment interrupt in caching-inhibited storage precedes the page-protection data storage interrupt when the entry's C bit is 1, and follows it when C is 0.
#[test]
fn a_caching_inhibited_store_to_a_read_only_page_orders_its_two_refusals_by_the_change_bit() {
    let alignment = (None, Some(irq::VECTOR_ALIGNMENT), false);
    let protection = (None, Some(irq::VECTOR_DATA_STORAGE), true);
    // Each case runs an unaligned sth, a stwcx. or a dcbz at page 4, which
    // is read-only and caching-inhibited at real 0x6000. A faulting store
    // runs first in its batch, so it always misses.
    // exec::mem::tests::change_bit holds the order on a view.
    for insn in [sth(4, 1, 5), stwcx(4, 5), dcbz(0, 5)] {
        for (c, want) in [(PTE_C, alignment), (0, protection)] {
            let mut mem = memory(&[insn]);
            map_page(&mut mem, 4, 0x6000, 0b11 | PTE_I | c);
            let mut unit = kernel_unit(MSR_SF | MSR_IR | MSR_DR);
            unit.state_mut().set_gpr(5, SEGMENT + 0x4000);
            let (r, _) = run(&mut unit, &mem, 1);
            assert_eq!(stop_class(&r), want, "insn {insn:#x} C {c:#x}");
        }
    }
}

/// [CBE-Handbook p:253 s:9.5.8] The alignment interrupt is taken when the data access cannot be performed in caching-inhibited storage, so it reaches no storage.
#[test]
fn an_access_the_ppe_refuses_in_caching_inhibited_storage_takes_that_refusal_where_no_storage_is() {
    let alignment = (None, Some(irq::VECTOR_ALIGNMENT), false);
    let protection = (None, Some(irq::VECTOR_DATA_STORAGE), true);
    let machine_check = (
        Some(FAULT_MACHINE_CHECK),
        Some(irq::VECTOR_MACHINE_CHECK),
        false,
    );
    // Page 4 is caching-inhibited at a real address no storage holds.
    // (instruction, PTE bits, stop).
    let cases = [
        (lwarx(4, 5), 0b10 | PTE_I, alignment),
        (stwcx(4, 5), 0b10 | PTE_I, alignment),
        (dcbz(0, 5), 0b10 | PTE_I, alignment),
        (lwz(4, 1, 5), 0b10 | PTE_I, alignment),
        (sth(4, 1, 5), 0b10 | PTE_I, alignment),
        (lwz(4, 0, 5), 0b10 | PTE_I, machine_check),
        // Read-only with C = 1: the aligned store meets the protection
        // before it would reach storage.
        (stw(4, 0, 5), 0b11 | PTE_I | PTE_C, protection),
        (sth(4, 1, 5), 0b11 | PTE_I | PTE_C, alignment),
    ];
    for (insn, pte, want) in cases {
        let mut mem = memory(&[insn]);
        map_page(&mut mem, 4, UNBACKED_RA, pte);
        let mut unit = kernel_unit(MSR_SF | MSR_IR | MSR_DR | MSR_ME);
        unit.state_mut().set_gpr(5, SEGMENT + 0x4000);
        let (r, _) = run(&mut unit, &mem, 1);
        assert_eq!(stop_class(&r), want, "insn {insn:#x} pte {pte:#x}");
    }
}

/// [CBE-Handbook p:290 s:9.10] The page-fault data storage interrupt outranks the alignment interrupt of caching-inhibited storage, and a store's page-protection interrupt follows it when C is 1 and precedes it when C is 0.
#[test]
fn an_unaligned_access_out_of_an_unbacked_inhibited_page_meets_the_next_page_first() {
    let alignment = (None, Some(irq::VECTOR_ALIGNMENT), false);
    let not_found = (None, Some(irq::VECTOR_DATA_STORAGE), false);
    let protection = (None, Some(irq::VECTOR_DATA_STORAGE), true);
    // Page 4 is caching-inhibited at a real address no storage holds.
    // Each operand starts in its last bytes and runs into page 5, which
    // is at real 0x7000 or has no entry. (instruction, page 5's PTE bits,
    // stop).
    let cases = [
        (lwz(4, 0xFFE, 5), Some(0b10), alignment),
        (lwz(4, 0xFFE, 5), None, not_found),
        (sth(4, 0xFFF, 5), Some(0b10), alignment),
        (sth(4, 0xFFF, 5), None, not_found),
        (sth(4, 0xFFF, 5), Some(0b11 | PTE_I | PTE_C), alignment),
        (sth(4, 0xFFF, 5), Some(0b11 | PTE_I), protection),
    ];
    for (insn, next, want) in cases {
        let mut mem = memory(&[insn]);
        map_page(&mut mem, 4, UNBACKED_RA, 0b10 | PTE_I);
        if let Some(pte) = next {
            map_page(&mut mem, 5, 0x7000, pte);
        }
        let mut unit = kernel_unit(MSR_SF | MSR_IR | MSR_DR | MSR_ME);
        unit.state_mut().set_gpr(5, SEGMENT + 0x4000);
        let (r, _) = run(&mut unit, &mem, 1);
        assert_eq!(stop_class(&r), want, "insn {insn:#x} page 5 {next:?}");
    }
}

/// [CBE-Handbook p:290 s:9.10] The page-fault data storage interrupt outranks a caching-inhibited store's alignment interrupt, which outranks its page-protection interrupt when C is 1.
#[test]
fn a_store_out_of_a_read_only_unbacked_inhibited_page_with_c_1_meets_the_next_page_first() {
    // Page 4 is read-only, caching-inhibited and C = 1 at a real address
    // no storage holds. The sth runs into page 5, which is read/write at
    // real 0x7000 or has no entry.
    let cases = [
        (Some(0b10), (None, Some(irq::VECTOR_ALIGNMENT), false)),
        (None, (None, Some(irq::VECTOR_DATA_STORAGE), false)),
    ];
    for (next, want) in cases {
        let mut mem = memory(&[sth(4, 0xFFF, 5)]);
        map_page(&mut mem, 4, UNBACKED_RA, 0b11 | PTE_I | PTE_C);
        if let Some(pte) = next {
            map_page(&mut mem, 5, 0x7000, pte);
        }
        let mut unit = kernel_unit(MSR_SF | MSR_IR | MSR_DR | MSR_ME);
        unit.state_mut().set_gpr(5, SEGMENT + 0x4000);
        let (r, _) = run(&mut unit, &mem, 1);
        assert_eq!(stop_class(&r), want, "page 5 {next:?}");
    }
}

/// [CBE-Handbook p:253 s:9.5.8] An unaligned load in caching-inhibited storage takes the alignment interrupt, whatever the page size.
#[test]
fn an_unaligned_load_inside_an_unbacked_large_inhibited_page_takes_the_alignment_interrupt() {
    use cellgov_ps3_abi::hw::ppu_mmu::{PPE_PTE_LP, PTE_L, SLB_L};
    use cellgov_ps3_abi::hw::ppu_supervisor::PPE_SLB_LP;
    // A segment of 64 KB pages (L = 1, LP = 1) at EA 0xD000_0000, VSID
    // 0x456. Its first page is caching-inhibited at a real address no
    // storage holds, in slot 0 of its primary group.
    const LARGE_EA: u64 = 0xD000_0000;
    const LARGE_VSID: u64 = 0x456;
    let mut mem = memory(&[lwz(4, 0xFFE, 5)]);
    let pteg = HTAB + (LARGE_VSID & 0x7FF) * 128;
    let d0 = (LARGE_VSID << 12) | PTE_L | 1;
    let d1 = UNBACKED_RA | PPE_PTE_LP | PTE_I | 0b10;
    put(&mut mem, pteg, &d0.to_be_bytes());
    put(&mut mem, pteg + 8, &d1.to_be_bytes());
    let mut unit = kernel_unit(MSR_SF | MSR_IR | MSR_DR | MSR_ME);
    unit.state_mut()
        .update_supervisor(|s| {
            s.slb[1] = SlbEntry {
                esid_v: LARGE_EA | SLB_VALID,
                vsid_flags: (LARGE_VSID << 12) | SLB_L | PPE_SLB_LP,
            };
        })
        .unwrap();
    unit.state_mut().set_gpr(5, LARGE_EA);
    let (r, _) = run(&mut unit, &mem, 1);
    assert_eq!(stop_class(&r), (None, Some(irq::VECTOR_ALIGNMENT), false));
}

/// [CBE-Handbook p:253 s:9.5.8] An unaligned operand in caching-inhibited storage takes the alignment interrupt, so the access reaches no storage.
#[test]
fn an_unaligned_access_with_one_inhibited_page_takes_alignment_where_the_other_is_unbacked() {
    let alignment = (None, Some(irq::VECTOR_ALIGNMENT), false);
    // Each operand starts in the last bytes of page 4 and runs into page
    // 5. One page is caching-inhibited; the other is not, and is at real
    // memory or at a real address no storage holds. (page 4's real
    // address and PTE bits, page 5's.)
    let layouts = [
        ((0x6000, 0b10 | PTE_I), (0x7000, 0b10)),
        ((0x6000, 0b10 | PTE_I), (UNBACKED_RA, 0b10)),
        ((0x6000, 0b10), (0x7000, 0b10 | PTE_I)),
        ((UNBACKED_RA, 0b10), (0x7000, 0b10 | PTE_I)),
    ];
    for insn in [lwz(4, 0xFFE, 5), sth(4, 0xFFF, 5)] {
        for ((ra4, pte4), (ra5, pte5)) in layouts {
            let mut mem = memory(&[insn]);
            map_page(&mut mem, 4, ra4, pte4);
            map_page(&mut mem, 5, ra5, pte5);
            let mut unit = kernel_unit(MSR_SF | MSR_IR | MSR_DR | MSR_ME);
            unit.state_mut().set_gpr(5, SEGMENT + 0x4000);
            let (r, _) = run(&mut unit, &mem, 1);
            assert_eq!(
                stop_class(&r),
                alignment,
                "insn {insn:#x} page 4 {ra4:#x}/{pte4:#x} page 5 {ra5:#x}/{pte5:#x}"
            );
        }
    }
}

#[test]
fn real_mode_runs_code_and_reads_data_in_a_region_above_real_address_0() {
    // A second region at real 1 MB, past the 512 KB at 0. Real mode
    // gives one block from real 0 that holds both regions: its code and
    // its data sit at their real addresses in that block.
    use cellgov_mem::{PageSize, Region};
    const HIGH: u64 = 0x10_0000;
    let ram = Region::new(0, 0x8_0000, "ram", PageSize::Page64K);
    let high = Region::new(HIGH, 0x1_0000, "high", PageSize::Page64K);
    let mut mem = GuestMemory::from_regions(vec![ram, high]).unwrap();
    put(&mut mem, HIGH, &lwz(4, 0x100, 5).to_be_bytes());
    put(&mut mem, HIGH + 0x100, &0x1357_9BDFu32.to_be_bytes());
    let mut unit = PpuExecutionUnit::new(UnitId::new(0));
    unit.state_mut()
        .set_supervisor(Some(SupervisorState::new(MSR_SF)));
    unit.state_mut().pc = HIGH;
    unit.state_mut().set_gpr(5, HIGH);
    let (r, effects) = run(&mut unit, &mem, 1);
    assert_eq!(
        r.yield_reason,
        YieldReason::BudgetExhausted,
        "{:?}",
        r.local_diagnostics
    );
    assert_eq!(unit.state().gpr[4], 0x1357_9BDF);
    let (reads, _) = ranges(&effects);
    assert!(reads.contains(&(HIGH + 0x100, 4)), "{reads:x?}");
    assert!(reads.contains(&(HIGH, 4)), "{reads:x?}");
}

#[test]
fn a_load_of_the_last_byte_of_the_page_below_the_last_reads_its_real_byte() {
    // Page 0xFFFE of the last segment, EA 2^64 - 8 KB, at real 0x7000:
    // the five high bits of its offset are 1, as for the last page.
    const BELOW_TOP: u64 = TOP_PAGE - 0x1000;
    let mut mem = memory(&[lbz(4, 0xFFF, 5)]);
    let mut unit = kernel_unit(MSR_SF | MSR_IR | MSR_DR);
    unit.state_mut()
        .update_supervisor(|s| {
            s.slb[2] = SlbEntry {
                esid_v: (BELOW_TOP & !0x0FFF_FFFF) | SLB_VALID,
                vsid_flags: VSID << 12,
            };
        })
        .unwrap();
    let pteg = HTAB + ((VSID ^ 0xFFFE) & 0x7FF) * 128;
    put(
        &mut mem,
        pteg,
        &((VSID << 12) | (0x1F << 7) | 1).to_be_bytes(),
    );
    put(&mut mem, pteg + 8, &(0x7000u64 | 0b10).to_be_bytes());
    put(&mut mem, 0x7FFF, &[0xAB]);
    unit.state_mut().set_gpr(5, BELOW_TOP);
    let (r, _) = run(&mut unit, &mem, 1);
    assert_eq!(
        r.yield_reason,
        YieldReason::BudgetExhausted,
        "{:?}",
        r.local_diagnostics
    );
    assert_eq!(unit.state().gpr[4], 0xAB);
}

#[test]
fn a_store_whose_second_page_has_a_cached_alias_ends_the_batch_before_a_load_through_it() {
    // Pages 2 and 6 both map FAR_RA. lwz r6 through page 6 caches it;
    // stw r4, 0xFFE(r5) writes two bytes at the end of page 1 and two at
    // the start of page 2; lwz r8 reads them back through page 6.
    let mut mem = memory(&[lwz(6, 0, 7), stw(4, 0xFFE, 5), lwz(8, 0, 7)]);
    map_page(&mut mem, 2, FAR_RA, 0b10);
    map_page(&mut mem, 6, FAR_RA, 0b10);
    let mut unit = kernel_unit(MSR_SF | MSR_IR | MSR_DR);
    unit.state_mut().set_gpr(4, 0x1234_5678);
    unit.state_mut().set_gpr(5, DATA_EA);
    unit.state_mut().set_gpr(7, SEGMENT + 0x6000);
    let (r, effects) = run(&mut unit, &mem, 3);
    assert_eq!(
        r.yield_reason,
        YieldReason::BudgetExhausted,
        "{:?}",
        r.local_diagnostics
    );
    assert_eq!(
        unit.retired_instructions(),
        2,
        "the batch ends at the store"
    );
    commit(&mut mem, &effects);
    let (r, _) = run(&mut unit, &mem, 1);
    assert_eq!(
        r.yield_reason,
        YieldReason::BudgetExhausted,
        "{:?}",
        r.local_diagnostics
    );
    assert_eq!(unit.state().gpr[8], 0x5678_0000);
}

#[test]
fn a_store_at_effective_address_0_through_a_cached_alias_ends_the_batch() {
    // Real mode: lwz r6 through the high alias of real 0 caches it, then
    // stw r4, 0(0) writes real 0 through the low one.
    let mem = memory(&[lwz(6, 0, 5), stw(4, 0, 0), NOP]);
    let mut unit = kernel_unit(MSR_SF);
    unit.state_mut().pc = 0x8000_0000_0000_3000;
    unit.state_mut().set_gpr(5, 0x8000_0000_0000_0000);
    let (r, effects) = run(&mut unit, &mem, 3);
    assert_eq!(
        r.yield_reason,
        YieldReason::BudgetExhausted,
        "{:?}",
        r.local_diagnostics
    );
    assert_eq!(
        unit.retired_instructions(),
        2,
        "the batch ends at the store"
    );
    assert_eq!(ranges(&effects).1, vec![(0, vec![0; 4])]);
}

#[test]
fn a_move_that_keeps_the_msr_keeps_the_batch_running() {
    let mem = memory(&[mtmsrd(7), lwz(4, 0, 5)]);
    let mut unit = kernel_unit(MSR_SF | MSR_IR | MSR_DR);
    unit.state_mut().set_gpr(7, MSR_SF | MSR_IR | MSR_DR);
    unit.state_mut().set_gpr(5, DATA_EA);
    let (r, _) = run(&mut unit, &mem, 2);
    assert_eq!(
        r.yield_reason,
        YieldReason::BudgetExhausted,
        "{:?}",
        r.local_diagnostics
    );
    assert_eq!(
        unit.retired_instructions(),
        2,
        "the move leaves the translation, so the batch runs on"
    );
    assert_eq!(unit.state().gpr[4], 0xDEAD_BEEF);
}
