//! A load that needs several sourced words of one reader gets each of
//! them in one batch, and a load that runs past its last sourced word
//! stops at the first byte no word holds.

use crate::fault_codes::FAULT_UNMODELLED_STORAGE;
use crate::mmu::Partition;
use crate::supervisor::{SlbEntry, SupervisorState};
use crate::*;
use cellgov_effects::FaultKind;
use cellgov_event::UnitId;
use cellgov_exec::{ExecutionContext, ExecutionUnit};
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
const INHIBITED: u64 = 0b10 | 0x20;
/// PP = 2 with no WIMG bit.
const CACHED: u64 = 0b10;
/// The offset of the first sourced word in the span. It is doubleword
/// aligned.
const OFFSET: u64 = 0x10;
const WORDS: [u32; 4] = [0x1122_3344, 0x5566_7788, 0x99AA_BBCC, 0xDDEE_FF00];

const TRANSLATED: u64 = MSR_SF | MSR_IR | MSR_DR | MSR_ME;

/// `ld r4,0(r5)`.
const LD: u32 = (58 << 26) | (4 << 21) | (5 << 16);
/// `lmw r28,0(r5)`.
const LMW: u32 = (46 << 26) | (28 << 21) | (5 << 16);
/// `li r6,1`.
const LI: u32 = (14 << 26) | (6 << 21) | 1;

/// The most batches a test runs before it reads the result.
const MAX_BATCHES: usize = 3;

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

/// 512 KB of memory with `code` on the code page and [`SPAN`] declared
/// unmodelled behind page [`PAGE`], whose entry has the low word `low`.
/// It declares the first `sourced` words of [`WORDS`] from [`OFFSET`] on,
/// each with the segment's first word as its reader.
fn memory(code: &[u32], low: u64, sourced: usize) -> GuestMemory {
    let mut mem = GuestMemory::new(0x8_0000);
    mem.declare_unmodelled(SPAN, SPAN_LEN).unwrap();
    for (i, word) in WORDS[..sourced].iter().enumerate() {
        mem.declare_sourced_word(SPAN + OFFSET + 4 * i as u64, *word, SEGMENT)
            .unwrap();
    }
    map_page(&mut mem, 0, CODE_RA, CACHED);
    map_page(&mut mem, PAGE, SPAN, low);
    for (i, word) in code.iter().enumerate() {
        put(&mut mem, CODE_RA + 4 * i as u64, &word.to_be_bytes());
    }
    mem
}

/// The unit at the segment's first word, with r4 and r6 at 0x5A and r5
/// at the first sourced word's EA.
fn unit() -> PpuExecutionUnit {
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
    unit
}

/// The fault of the last batch, its faulting EA and RA, and the unit.
type Run = (
    (Option<FaultKind>, Option<u64>, Option<u64>),
    PpuExecutionUnit,
);

/// [`run_unit`] with [`unit`].
fn run(mem: &GuestMemory) -> Run {
    run_unit(mem, unit())
}

/// Runs batches of 2 instructions of `unit`, at most [`MAX_BATCHES`].
///
/// The run stops early after a batch that ends in one of these states:
/// - a fault;
/// - the pc past the segment's first word.
fn run_unit(mem: &GuestMemory, mut unit: PpuExecutionUnit) -> Run {
    let ctx = ExecutionContext::new(mem);
    let mut stop = (None, None, None);
    for _ in 0..MAX_BATCHES {
        let mut effects = Vec::new();
        let r = unit.run_until_yield(Budget::new(2), &ctx, &mut effects);
        let d = &r.local_diagnostics;
        stop = (r.fault, d.faulting_ea, d.faulting_ra);
        if r.fault.is_some() || unit.state().pc != SEGMENT {
            break;
        }
    }
    (stop, unit)
}

#[test]
fn an_aligned_ld_over_two_sourced_words_of_its_reader_gets_both_words() {
    let mem = memory(&[LD, LI], INHIBITED, 2);
    let (stop, unit) = run(&mem);
    assert_eq!(stop, (None, None, None));
    let gpr = unit.state().gpr.as_array();
    let both = (u64::from(WORDS[0]) << 32) | u64::from(WORDS[1]);
    assert_eq!((gpr[4], gpr[6]), (both, 0x5A));
    assert_eq!(unit.state().pc, SEGMENT + 4);
}

#[test]
fn an_lmw_over_four_sourced_words_of_its_reader_gets_each_word() {
    let mem = memory(&[LMW, LI], CACHED, 4);
    let (stop, unit) = run(&mem);
    assert_eq!(stop, (None, None, None));
    let gpr = unit.state().gpr.as_array();
    let loaded = [gpr[28], gpr[29], gpr[30], gpr[31]];
    assert_eq!(loaded, WORDS.map(u64::from));
    assert_eq!(unit.state().pc, SEGMENT + 4);
}

#[test]
fn an_lmw_over_sourced_words_on_two_pages_gets_each_word() {
    // The span has two pages, behind pages PAGE and PAGE + 1 of the
    // segment. Two words end the first page and two start the second, so
    // the load misses on the second page after the words of the first
    // answered it.
    let mut mem = GuestMemory::new(0x8_0000);
    mem.declare_unmodelled(SPAN, 2 * SPAN_LEN).unwrap();
    let first = SPAN + SPAN_LEN - 8;
    for (i, word) in WORDS.iter().enumerate() {
        mem.declare_sourced_word(first + 4 * i as u64, *word, SEGMENT)
            .unwrap();
    }
    map_page(&mut mem, 0, CODE_RA, CACHED);
    map_page(&mut mem, PAGE, SPAN, CACHED);
    map_page(&mut mem, PAGE + 1, SPAN + SPAN_LEN, CACHED);
    for (i, word) in [LMW, LI].iter().enumerate() {
        put(&mut mem, CODE_RA + 4 * i as u64, &word.to_be_bytes());
    }
    let mut unit = unit();
    unit.state_mut().set_gpr(5, PAGE_EA + SPAN_LEN - 8);
    let (stop, unit) = run_unit(&mem, unit);
    assert_eq!(stop, (None, None, None));
    let gpr = unit.state().gpr.as_array();
    let loaded = [gpr[28], gpr[29], gpr[30], gpr[31]];
    assert_eq!(loaded, WORDS.map(u64::from));
    assert_eq!(unit.state().pc, SEGMENT + 4);
}

#[test]
fn an_ld_past_its_readers_one_sourced_word_stops_at_the_next_word() {
    let mem = memory(&[LD, LI], INHIBITED, 1);
    let (stop, unit) = run(&mem);
    let next = (
        Some(FaultKind::Guest(FAULT_UNMODELLED_STORAGE)),
        Some(PAGE_EA + OFFSET + 4),
        Some(SPAN + OFFSET + 4),
    );
    assert_eq!(stop, next);
    assert_eq!(unit.state().gpr.as_array()[4], 0x5A);
    assert_eq!(unit.state().pc, SEGMENT);
}
