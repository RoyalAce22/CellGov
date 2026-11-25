//! A translated access to an unmodelled span of the memory stops there
//! with its addresses. An access to a real address no storage holds
//! keeps the checkstop and the machine check.

use crate::exec::PpuFault;
use crate::fault_codes::{FAULT_CHECKSTOP, FAULT_MACHINE_CHECK, FAULT_UNMODELLED_STORAGE};
use crate::mmu::Partition;
use crate::supervisor::{SlbEntry, SupervisorState};
use crate::*;
use cellgov_effects::{Effect, FaultKind};
use cellgov_event::UnitId;
use cellgov_exec::{ExecutionContext, ExecutionStepResult, ExecutionUnit};
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_ps3_abi::hw::ppu_interrupt as irq;
use cellgov_ps3_abi::hw::ppu_supervisor::{MSR_DR, MSR_IR, MSR_ME, MSR_SF, SLB_VALID};
use cellgov_time::Budget;

/// The page table: 2^11 groups from 256 KB.
const HTAB: u64 = 0x4_0000;
/// The segment the SLB maps: EA 0xC000_0000, VSID 0x123.
const SEGMENT: u64 = 0xC000_0000;
const VSID: u64 = 0x123;
/// The code page: page 0 of the segment, at real 0x3000.
const CODE_RA: u64 = 0x3000;
/// The page of the segment that the accesses reach: page 4.
const PAGE: u64 = 4;
const PAGE_EA: u64 = SEGMENT + (PAGE << 12);
/// The unmodelled span: one 4 KB page past the 512 KB the memory holds.
const SPAN: u64 = 0x9_0000;
const SPAN_LEN: u64 = 0x1000;
/// PP = 2: loads and stores for both keys.
const PP_READ_WRITE: u64 = 0b10;
/// The PTE caching-inhibited bit.
const PTE_I: u64 = 0x20;
/// The offset of each access in its page.
const OFFSET: u64 = 0x10;

fn d_form(op: u32, rt: u32, d: u16, ra: u32) -> u32 {
    (op << 26) | (rt << 21) | (ra << 16) | u32::from(d)
}

/// Each load of `r4` from `0(r5)`, by width: `lbz`, `lhz`, `lwz`, `ld`.
const LOADS: [u32; 4] = [
    (34 << 26) | (4 << 21) | (5 << 16),
    (40 << 26) | (4 << 21) | (5 << 16),
    (32 << 26) | (4 << 21) | (5 << 16),
    (58 << 26) | (4 << 21) | (5 << 16),
];

/// Each store of `r4` to `0(r5)`, by width: `stb`, `sth`, `stw`, `std`.
const STORES: [u32; 4] = [
    (38 << 26) | (4 << 21) | (5 << 16),
    (44 << 26) | (4 << 21) | (5 << 16),
    (36 << 26) | (4 << 21) | (5 << 16),
    (62 << 26) | (4 << 21) | (5 << 16),
];

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

/// 512 KB of memory with the page table, `code` on the code page and
/// [`SPAN`] declared unmodelled. Page [`PAGE`] maps real `page_ra` with
/// the PTE bits `low`.
fn memory(code: &[u32], page_ra: u64, low: u64) -> GuestMemory {
    layout(0x8_0000, SPAN, code, page_ra, low)
}

/// [`memory`] with `size` bytes of memory and the span at `span`.
fn layout(size: u64, span: u64, code: &[u32], page_ra: u64, low: u64) -> GuestMemory {
    let mut mem = GuestMemory::new(size as usize);
    mem.declare_unmodelled(span, SPAN_LEN).unwrap();
    map_page(&mut mem, 0, CODE_RA, PP_READ_WRITE);
    map_page(&mut mem, PAGE, page_ra, low);
    for (i, word) in code.iter().enumerate() {
        put(&mut mem, CODE_RA + 4 * i as u64, &word.to_be_bytes());
    }
    mem
}

/// A unit at the segment's first word with MSR `msr`, SLB entry 0 over
/// the segment and the page table at [`HTAB`].
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

/// Runs one instruction `insn` at the segment's first word with r5 at
/// [`PAGE_EA`] plus [`OFFSET`].
fn run(insn: u32, page_ra: u64, low: u64, msr: u64) -> (ExecutionStepResult, Vec<Effect>) {
    let (r, effects, _) = run_in(&memory(&[insn], page_ra, low), msr);
    (r, effects)
}

/// The value of r28 to r31 before each run.
const SENTINEL: u64 = 0x5A5A_5A5A_5A5A_5A5A;

/// [`run`] on `mem`, with r28 to r31 at [`SENTINEL`]. Also gives the unit.
fn run_in(mem: &GuestMemory, msr: u64) -> (ExecutionStepResult, Vec<Effect>, PpuExecutionUnit) {
    let mut unit = kernel_unit(msr);
    unit.state_mut().set_gpr(4, 0x0102_0304_0506_0708);
    unit.state_mut().set_gpr(5, PAGE_EA + OFFSET);
    for n in 28..32 {
        unit.state_mut().set_gpr(n, SENTINEL);
    }
    let mut effects = Vec::new();
    let r = unit.run_until_yield(Budget::new(1), &ExecutionContext::new(mem), &mut effects);
    (r, effects, unit)
}

/// The run's fault code, faulting EA, faulting real address and
/// interrupt vector.
fn stop(r: &ExecutionStepResult) -> (Option<FaultKind>, Option<u64>, Option<u64>, Option<u64>) {
    let d = &r.local_diagnostics;
    (
        r.fault,
        d.faulting_ea,
        d.faulting_ra,
        d.interrupt.map(|i| i.vector),
    )
}

/// The start addresses of the write intents outside the page table.
fn stores(effects: &[Effect]) -> Vec<u64> {
    effects
        .iter()
        .filter_map(|e| match e {
            Effect::SharedWriteIntent { range, .. } => Some(range.start().raw()),
            _ => None,
        })
        .filter(|at| !(HTAB..HTAB + 0x4_0000).contains(at))
        .collect()
}

const TRANSLATED: u64 = MSR_SF | MSR_IR | MSR_DR;

fn unmodelled_stop() -> (Option<FaultKind>, Option<u64>, Option<u64>, Option<u64>) {
    (
        Some(FaultKind::Guest(FAULT_UNMODELLED_STORAGE)),
        Some(PAGE_EA + OFFSET),
        Some(SPAN + OFFSET),
        None,
    )
}

#[test]
fn every_load_width_to_an_unmodelled_span_stops_there_with_its_addresses() {
    let mut runs = 0;
    for insn in LOADS {
        for low in [PP_READ_WRITE, PP_READ_WRITE | PTE_I] {
            for me in [0, MSR_ME] {
                let (r, effects) = run(insn, SPAN, low, TRANSLATED | me);
                let case = format!("insn {insn:#x} pte {low:#x} me {me:#x}");
                assert_eq!(stop(&r), unmodelled_stop(), "{case}");
                assert_eq!(stores(&effects), Vec::<u64>::new(), "{case}");
                runs += 1;
            }
        }
    }
    assert_eq!(runs, 16);
}

#[test]
fn every_store_width_to_an_unmodelled_span_stops_there_and_publishes_no_write() {
    let mut runs = 0;
    for insn in STORES {
        for low in [PP_READ_WRITE, PP_READ_WRITE | PTE_I] {
            let (r, effects) = run(insn, SPAN, low, TRANSLATED | MSR_ME);
            let case = format!("insn {insn:#x} pte {low:#x}");
            assert_eq!(stop(&r), unmodelled_stop(), "{case}");
            assert_eq!(stores(&effects), Vec::<u64>::new(), "{case}");
            runs += 1;
        }
    }
    assert_eq!(runs, 8);
}

#[test]
fn a_fetch_from_an_unmodelled_span_stops_there_with_its_addresses() {
    let mem = memory(&[], SPAN, PP_READ_WRITE);
    let mut unit = kernel_unit(MSR_SF | MSR_IR | MSR_ME);
    unit.state_mut().pc = PAGE_EA + OFFSET;
    let mut effects = Vec::new();
    let r = unit.run_until_yield(Budget::new(1), &ExecutionContext::new(&mem), &mut effects);
    assert_eq!(stop(&r), unmodelled_stop());
}

/// [PPC-Book3 p:66 s:5.5.2] An access to nonexistent storage may enter the checkstop state; [CBE-Handbook p:247 s:9.5.2] the PPE's one machine check source is a caching-inhibited load.
#[test]
fn the_first_byte_past_an_unmodelled_span_keeps_the_checkstop_and_the_machine_check() {
    // The displacement -OFFSET puts each access on the page's first byte.
    let back = (OFFSET as u16).wrapping_neg();
    let lwz = d_form(32, 4, back, 5);
    let stw = d_form(36, 4, back, 5);
    let checkstop = (Some(FaultKind::Guest(FAULT_CHECKSTOP)), None, None, None);
    let machine_check = (
        Some(FaultKind::Guest(FAULT_MACHINE_CHECK)),
        None,
        None,
        Some(irq::VECTOR_MACHINE_CHECK),
    );
    // The page after the span, so the access's real address is
    // SPAN + SPAN_LEN, the first byte that the span does not hold.
    let past = SPAN + SPAN_LEN;
    // (instruction, PTE bits, MSR[ME], stop).
    let cases = [
        (lwz, PP_READ_WRITE | PTE_I, MSR_ME, machine_check),
        (lwz, PP_READ_WRITE | PTE_I, 0, checkstop),
        (lwz, PP_READ_WRITE, MSR_ME, checkstop),
        (stw, PP_READ_WRITE | PTE_I, MSR_ME, checkstop),
    ];
    for (insn, low, me, want) in cases {
        let (r, _) = run(insn, past, low, TRANSLATED | me);
        assert_eq!(stop(&r), want, "insn {insn:#x} pte {low:#x} me {me:#x}");
    }
}

#[test]
fn an_unmodelled_storage_stop_is_a_cellgov_stop_of_the_unmodelled_class() {
    let fault = PpuFault::UnmodelledStorage {
        ea: PAGE_EA,
        ra: SPAN,
    };
    // The class half of the unmodelled stop with detail half 1.
    assert_eq!(FAULT_UNMODELLED_STORAGE, 0x0117_0001);
    assert_eq!(fault.guest_code(), FAULT_UNMODELLED_STORAGE);
    assert_eq!(fault.class(), PpuFaultClass::Unmodelled);
    assert_eq!(
        PpuFaultClass::of_code(FAULT_UNMODELLED_STORAGE),
        Some(PpuFaultClass::Unmodelled)
    );
    assert!(!fault.class().is_interrupt());
}

/// The end of the region in the straddle cases, half way into the real
/// page that page [`PAGE`] maps. The span starts here.
const EDGE: u64 = 0x8_0800;
const EDGE_PAGE_RA: u64 = 0x8_0000;

/// Runs `stmw r28` (opcode 47) or `lmw r28` (opcode 46) at the EA 8 bytes
/// below [`EDGE`]: two words in the region, which holds nonzero bytes,
/// and two in the span.
fn straddle(op: u32) -> (ExecutionStepResult, Vec<Effect>, PpuExecutionUnit) {
    let d = ((EDGE - EDGE_PAGE_RA - 8) as u16).wrapping_sub(OFFSET as u16);
    let mut mem = layout(
        EDGE,
        EDGE,
        &[d_form(op, 28, d, 5)],
        EDGE_PAGE_RA,
        PP_READ_WRITE,
    );
    put(&mut mem, EDGE - 8, &[0xA5; 8]);
    run_in(&mem, TRANSLATED | MSR_ME)
}

/// The stop at the span's first byte.
fn straddle_stop() -> (Option<FaultKind>, Option<u64>, Option<u64>, Option<u64>) {
    (
        Some(FaultKind::Guest(FAULT_UNMODELLED_STORAGE)),
        Some(PAGE_EA + (EDGE - EDGE_PAGE_RA)),
        Some(EDGE),
        None,
    )
}

#[test]
fn a_store_multiple_from_a_region_into_a_span_stops_at_the_spans_first_byte_and_publishes_no_write()
{
    let (r, effects, _) = straddle(47);
    assert_eq!(stop(&r), straddle_stop());
    assert_eq!(stores(&effects), Vec::<u64>::new());
}

#[test]
fn a_load_multiple_from_a_region_into_a_span_stops_at_the_spans_first_byte_and_loads_no_register() {
    let (r, _, unit) = straddle(46);
    assert_eq!(stop(&r), straddle_stop());
    assert_eq!(&unit.state().gpr.as_array()[28..], &[SENTINEL; 4]);
}

/// [CBE-Handbook p:253 s:9.5.8] An stmw operand in caching-inhibited storage takes the alignment interrupt, aligned or not.
#[test]
fn a_store_multiple_to_a_caching_inhibited_span_takes_the_alignment_interrupt_first() {
    let stmw = d_form(47, 28, 0, 5);
    let (r, effects) = run(stmw, SPAN, PP_READ_WRITE | PTE_I, TRANSLATED | MSR_ME);
    let interrupt = r.local_diagnostics.interrupt;
    assert_eq!(
        (r.fault, r.local_diagnostics.faulting_ra),
        (None, None),
        "the thread takes the interrupt and does not stop"
    );
    assert_eq!(
        interrupt.map(|i| (i.vector, i.dar)),
        Some((irq::VECTOR_ALIGNMENT, Some(PAGE_EA + OFFSET)))
    );
    assert_eq!(stores(&effects), Vec::<u64>::new());
}
