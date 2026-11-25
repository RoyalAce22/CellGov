//! A sourced word of an unmodelled span answers the load of its reader
//! only, and the batch ends after that load.

use crate::fault_codes::FAULT_UNMODELLED_STORAGE;
use crate::instruction::{FusedPair, PpuInstruction};
use crate::mmu::{Access, Partition, Translation};
use crate::supervisor::{SlbEntry, SupervisorState};
use crate::*;
use cellgov_effects::FaultKind;
use cellgov_event::UnitId;
use cellgov_exec::{ExecutionContext, ExecutionStepResult, ExecutionUnit};
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_ps3_abi::hw::ppu_supervisor::{MSR_DR, MSR_IR, MSR_ME, MSR_SF, SLB_VALID};
use cellgov_time::Budget;

/// The page table: 2^11 groups from 256 KB.
const HTAB: u64 = 0x4_0000;
/// The segment the SLB maps: EA 0xC000_0000, VSID 0x123.
const SEGMENT: u64 = 0xC000_0000;
const VSID: u64 = 0x123;
/// The code page: page 0 of the segment, at real 0x3000.
const CODE_RA: u64 = 0x3000;
/// The page of the segment that maps the span: page 4.
const PAGE: u64 = 4;
const PAGE_EA: u64 = SEGMENT + (PAGE << 12);
/// The unmodelled span: one 4 KB page past the 512 KB the memory holds.
const SPAN: u64 = 0x9_0000;
const SPAN_LEN: u64 = 0x1000;
/// PP = 2 with the caching-inhibited bit, as a device's MMIO page has.
const PTE_LOW: u64 = 0b10 | 0x20;
/// The offset of the sourced word in the span.
const OFFSET: u64 = 0x10;
const WORD: u32 = 0x0000_0010;

const TRANSLATED: u64 = MSR_SF | MSR_IR | MSR_DR | MSR_ME;

/// `lwz r4,0(r5)`.
const LWZ: u32 = (32 << 26) | (4 << 21) | (5 << 16);
/// `stw r4,0(r5)`.
const STW: u32 = (36 << 26) | (4 << 21) | (5 << 16);
/// `li r6,1`.
const LI: u32 = (14 << 26) | (6 << 21) | 1;

fn put(mem: &mut GuestMemory, at: u64, bytes: &[u8]) {
    let range = ByteRange::new(GuestAddr::new(at), bytes.len() as u64).unwrap();
    mem.apply_commit(range, bytes).unwrap();
}

/// Writes the entry for page `index` of the segment to slot 0 of its
/// primary group.
fn map_page(mem: &mut GuestMemory, index: u64, ra: u64, low: u64) {
    let pteg = HTAB + ((VSID ^ index) & 0x7FF) * 128;
    put(mem, pteg, &((VSID << 12) | 1).to_be_bytes());
    put(mem, pteg + 8, &(ra | low).to_be_bytes());
}

/// 512 KB of memory with `code` on the code page, [`SPAN`] declared
/// unmodelled behind page [`PAGE`], and [`WORD`] sourced at
/// [`OFFSET`] for the instruction at `reader`.
fn memory(code: &[u32], reader: u64) -> GuestMemory {
    let mut mem = GuestMemory::new(0x8_0000);
    mem.declare_unmodelled(SPAN, SPAN_LEN).unwrap();
    mem.declare_sourced_word(SPAN + OFFSET, WORD, reader)
        .unwrap();
    map_page(&mut mem, 0, CODE_RA, 0b10);
    map_page(&mut mem, PAGE, SPAN, PTE_LOW);
    for (i, word) in code.iter().enumerate() {
        put(&mut mem, CODE_RA + 4 * i as u64, &word.to_be_bytes());
    }
    mem
}

/// Runs a batch of `budget` instructions from the segment's first word,
/// with r4 and r6 at 0x5A and r5 at the sourced word's EA.
fn run(mem: &GuestMemory, budget: u64) -> (ExecutionStepResult, PpuExecutionUnit) {
    let mut sup = SupervisorState::new(TRANSLATED);
    sup.slb[0] = SlbEntry {
        esid_v: SEGMENT | SLB_VALID,
        vsid_flags: VSID << 12,
    };
    sup.partition = Partition::new(HTAB, 1 << 62, 0, Partition::INITIAL.lb()).unwrap();
    let mut unit = PpuExecutionUnit::new(UnitId::new(0));
    unit.state_mut().set_supervisor(Some(sup));
    unit.state_mut().pc = SEGMENT;
    unit.state_mut().set_gpr(4, 0x5A);
    unit.state_mut().set_gpr(5, PAGE_EA + OFFSET);
    unit.state_mut().set_gpr(6, 0x5A);
    let mut effects = Vec::new();
    let ctx = ExecutionContext::new(mem);
    let r = unit.run_until_yield(Budget::new(budget), &ctx, &mut effects);
    (r, unit)
}

fn unmodelled_stop(r: &ExecutionStepResult) -> (Option<FaultKind>, Option<u64>, Option<u64>) {
    let d = &r.local_diagnostics;
    (r.fault, d.faulting_ea, d.faulting_ra)
}

const STOP_AT_WORD: (Option<FaultKind>, Option<u64>, Option<u64>) = (
    Some(FaultKind::Guest(FAULT_UNMODELLED_STORAGE)),
    Some(PAGE_EA + OFFSET),
    Some(SPAN + OFFSET),
);

#[test]
fn the_readers_load_gets_the_sourced_word_and_its_batch_ends_after_it() {
    let mem = memory(&[LWZ, LI], SEGMENT);
    let (r, unit) = run(&mem, 2);
    assert_eq!(r.fault, None);
    let gpr = unit.state().gpr.as_array();
    assert_eq!((gpr[4], gpr[6]), (u64::from(WORD), 0x5A));
    assert_eq!(unit.state().pc, SEGMENT + 4);
}

#[test]
fn the_batch_after_the_readers_load_runs_its_whole_budget() {
    let mem = memory(&[LWZ, LI, LI], SEGMENT);
    let (first, mut unit) = run(&mem, 2);
    assert_eq!((first.fault, unit.state().pc), (None, SEGMENT + 4));
    let mut effects = Vec::new();
    let ctx = ExecutionContext::new(&mem);
    let r = unit.run_until_yield(Budget::new(2), &ctx, &mut effects);
    assert_eq!(r.fault, None);
    assert_eq!(unit.state().pc, SEGMENT + 12);
}

#[test]
fn a_load_by_another_instruction_stops_at_the_sourced_word() {
    let mem = memory(&[LWZ], SEGMENT + 4);
    let (r, unit) = run(&mem, 1);
    assert_eq!(unmodelled_stop(&r), STOP_AT_WORD);
    assert_eq!(unit.state().gpr.as_array()[4], 0x5A);
}

#[test]
fn a_store_by_the_reader_stops_at_the_sourced_word() {
    let mem = memory(&[STW], SEGMENT);
    let (r, _) = run(&mem, 1);
    assert_eq!(unmodelled_stop(&r), STOP_AT_WORD);
}

#[test]
fn a_fused_pair_gets_no_sourced_word_for_its_own_address() {
    let mem = memory(&[], SEGMENT);
    let t = Translation {
        ea: PAGE_EA,
        ra: SPAN,
        len: SPAN_LEN,
        writable: true,
        inhibited: true,
        guarded: false,
        changed: true,
    };
    let at = (SPAN + OFFSET, Access::Load);
    let lwz = crate::decode::decode(LWZ).unwrap();
    let pair = PpuInstruction::Fused(FusedPair::LwzLwz {
        rt1: 4,
        ra1: 5,
        off1: 0,
        rt2: 6,
        ra2: 5,
        off2: 4,
    });
    let view = PpuExecutionUnit::sourced_view(&lwz, &mem, &t, at, SEGMENT).unwrap();
    assert_eq!((view.base, view.real), (PAGE_EA + OFFSET, SPAN + OFFSET));
    assert_eq!(view.bytes, WORD.to_be_bytes());
    assert!(PpuExecutionUnit::sourced_view(&pair, &mem, &t, at, SEGMENT).is_none());
}
