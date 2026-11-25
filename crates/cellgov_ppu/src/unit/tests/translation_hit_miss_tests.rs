//! A translating thread's access gives one result whether its
//! translation is already cached or the access misses and runs again.

use crate::fault_codes::FAULT_CHECKSTOP;
use crate::mmu::Partition;
use crate::supervisor::{SlbEntry, SupervisorState};
use crate::*;
use cellgov_event::UnitId;
use cellgov_exec::{ExecutionContext, ExecutionStepResult, ExecutionUnit, YieldReason};
use cellgov_mem::{
    ByteRange, GuestAddr, GuestMemory, PageSize, ProvisionalRead, Region, RegionAccess,
};
use cellgov_ps3_abi::hw::ppu_supervisor::{MSR_DR, MSR_IR, MSR_SF, SLB_VALID};
use cellgov_time::Budget;

/// The page table: 2^11 groups at 256 KB.
const HTAB: u64 = 0x4_0000;
/// The segment the SLB maps: EA 0xC000_0000, VSID 0x123.
const SEGMENT: u64 = 0xC000_0000;
const VSID: u64 = 0x123;
/// The zero-readable region, which page 1 of the segment maps.
const ZERO_RA: u64 = 0x10_0000;

fn lwz(rt: u32, d: u16, ra: u32) -> u32 {
    (32 << 26) | (rt << 21) | (ra << 16) | u32::from(d)
}

fn lmw(rt: u32, d: u16, ra: u32) -> u32 {
    (46 << 26) | (rt << 21) | (ra << 16) | u32::from(d)
}

fn lwarx(rt: u32, ra: u32, rb: u32) -> u32 {
    (31 << 26) | (rt << 21) | (ra << 16) | (rb << 11) | (20 << 1)
}

fn stw(rs: u32, d: u16, ra: u32) -> u32 {
    (36 << 26) | (rs << 21) | (ra << 16) | u32::from(d)
}

fn put(mem: &mut GuestMemory, at: u64, bytes: &[u8]) {
    let range = ByteRange::new(GuestAddr::new(at), bytes.len() as u64).unwrap();
    mem.apply_commit(range, bytes).unwrap();
}

/// Slot 0 of the primary group for page `index` of the segment, read and
/// write.
fn map_page(mem: &mut GuestMemory, index: u64, ra: u64) {
    let pteg = HTAB + ((VSID ^ index) & 0x7FF) * 128;
    put(mem, pteg, &((VSID << 12) | 1).to_be_bytes());
    put(mem, pteg + 8, &(ra | 0b10).to_be_bytes());
}

fn run(unit: &mut PpuExecutionUnit, mem: &GuestMemory, budget: u64) -> ExecutionStepResult {
    let mut effects = Vec::new();
    unit.run_until_yield(
        Budget::new(budget),
        &ExecutionContext::new(mem),
        &mut effects,
    )
}

/// The guest fault code a run stopped with, and the PC it stopped at.
fn stopped(unit: &PpuExecutionUnit, r: &ExecutionStepResult) -> (Option<u32>, u64) {
    let code = match r.fault {
        Some(cellgov_effects::FaultKind::Guest(code)) => Some(code),
        _ => None,
    };
    (code, unit.state().pc)
}

/// [PPC-Book3 p:66 s:5.5.2] An access to nonexistent storage may enter the checkstop state.
#[test]
fn a_load_from_no_storage_through_a_cached_translation_stops_as_the_miss_does() {
    // Real mode, 512 KB of memory. Cached: `lmw r28, 0(r5)` at 0x3000 with
    // r5 at 0x7_FFF8 misses, caches the one real mode translation and
    // runs again; its third word, real 0x8_0000, then reaches no storage
    // through that cached translation. Missed: `lwz r6, 0(r7)` at 0x3004
    // with r7 at 0x8_0000 misses and its translation reaches no storage.
    let mut mem = GuestMemory::new(0x8_0000);
    put(&mut mem, 0x3000, &lmw(28, 0, 5).to_be_bytes());
    put(&mut mem, 0x3004, &lwz(6, 0, 7).to_be_bytes());
    let run_at = |pc| {
        let mut unit = PpuExecutionUnit::new(UnitId::new(0));
        unit.state_mut()
            .set_supervisor(Some(SupervisorState::new(MSR_SF)));
        unit.state_mut().pc = pc;
        unit.state_mut().set_gpr(5, 0x7_FFF8);
        unit.state_mut().set_gpr(7, 0x8_0000);
        let r = run(&mut unit, &mem, 1);
        stopped(&unit, &r)
    };
    let hit = run_at(0x3000);
    let miss = run_at(0x3004);
    assert_eq!(miss, (Some(FAULT_CHECKSTOP), 0x3004));
    assert_eq!(
        hit,
        (Some(FAULT_CHECKSTOP), 0x3000),
        "the cached translation gives the miss's stop"
    );
}

/// A real mode unit at 0x3000 with the words `code` there.
fn real_mode_unit(mem: &mut GuestMemory, code: &[u32]) -> PpuExecutionUnit {
    for (i, word) in code.iter().enumerate() {
        put(mem, 0x3000 + 4 * i as u64, &word.to_be_bytes());
    }
    let mut unit = PpuExecutionUnit::new(UnitId::new(0));
    unit.state_mut()
        .set_supervisor(Some(SupervisorState::new(MSR_SF)));
    unit.state_mut().pc = 0x3000;
    unit
}

#[test]
fn an_lwarx_run_again_after_a_translation_miss_counts_one_execution() {
    // `lwarx r6, 0, r7` with r7 at 0x5000 misses in the empty cache,
    // caches the real mode translation and runs again.
    let mut mem = GuestMemory::new(0x8_0000);
    let mut unit = real_mode_unit(&mut mem, &[lwarx(6, 0, 7)]);
    unit.state_mut().set_gpr(7, 0x5000);
    let r = run(&mut unit, &mem, 1);
    assert_eq!(
        r.yield_reason,
        YieldReason::BudgetExhausted,
        "{:?}",
        r.local_diagnostics
    );
    assert_eq!(unit.retired_instructions(), 1);
    assert_eq!(
        unit.state().lwarx_executed,
        1,
        "the rerun is the same execution"
    );
}

#[test]
fn an_lwarx_that_ends_the_batch_at_an_alias_counts_no_execution_until_it_runs() {
    // `stw r8, 0(r7)` buffers a store to real 0x5000. `lwarx r6, 0, r9`
    // with r9 at EA 0x4000_0000_0000_5000 then misses, and its real mode
    // translation reaches the store's real bytes: the batch ends before
    // the lwarx runs.
    let mut mem = GuestMemory::new(0x8_0000);
    let mut unit = real_mode_unit(&mut mem, &[stw(8, 0, 7), lwarx(6, 0, 9)]);
    unit.state_mut().set_gpr(7, 0x5000);
    unit.state_mut().set_gpr(9, 0x4000_0000_0000_5000);
    let r = run(&mut unit, &mem, 2);
    assert_eq!(
        r.yield_reason,
        YieldReason::BudgetExhausted,
        "{:?}",
        r.local_diagnostics
    );
    assert_eq!((unit.retired_instructions(), unit.state().pc), (1, 0x3004));
    assert_eq!(
        unit.state().lwarx_executed,
        0,
        "the lwarx counts when the next batch runs it"
    );
}

/// The zero-readable reads that one run of the `lmw` at the segment's
/// first word logs.
fn lmw_reads(unit: &mut PpuExecutionUnit, mem: &GuestMemory) -> Vec<ProvisionalRead> {
    unit.state_mut().pc = SEGMENT;
    let r = run(unit, mem, 1);
    assert_eq!(
        r.yield_reason,
        YieldReason::BudgetExhausted,
        "{:?}",
        r.local_diagnostics
    );
    mem.drain_provisional_reads()
}

#[test]
fn an_instruction_run_again_after_a_translation_miss_logs_each_provisional_read_once() {
    // lmw r28, 0xFF8(r5) with r5 at page 1: two words in the zero-readable
    // page, then two in page 2. Each page misses in turn on the first run.
    let ram = Region::new(0, 0x8_0000, "ram", PageSize::Page64K);
    let zero = Region::with_access(
        ZERO_RA,
        0x1000,
        "zero",
        PageSize::Page4K,
        RegionAccess::ReservedZeroReadable,
    );
    let mut mem = GuestMemory::from_regions(vec![ram, zero]).unwrap();
    map_page(&mut mem, 0, 0x3000);
    map_page(&mut mem, 1, ZERO_RA);
    map_page(&mut mem, 2, 0x5000);
    put(&mut mem, 0x3000, &lmw(28, 0xFF8, 5).to_be_bytes());
    let mut sup = SupervisorState::new(MSR_SF | MSR_IR | MSR_DR);
    sup.slb[0] = SlbEntry {
        esid_v: SEGMENT | SLB_VALID,
        vsid_flags: VSID << 12,
    };
    sup.partition = Partition::new(HTAB, 1 << 62, 0, Partition::INITIAL.lb()).unwrap();
    let mut unit = PpuExecutionUnit::new(UnitId::new(0));
    unit.state_mut().set_supervisor(Some(sup));
    unit.state_mut().set_gpr(5, SEGMENT + 0x1000);
    // A translation lives for one batch, so each run misses on both
    // pages and runs the lmw again after each miss.
    let first = lmw_reads(&mut unit, &mem);
    let again = lmw_reads(&mut unit, &mem);
    let once = |addr| ProvisionalRead {
        addr,
        len: 4,
        hits: 1,
    };
    assert_eq!(
        first,
        [once(ZERO_RA + 0xFF8), once(ZERO_RA + 0xFFC)],
        "the reruns took back their reads"
    );
    assert_eq!(again, first);
}
