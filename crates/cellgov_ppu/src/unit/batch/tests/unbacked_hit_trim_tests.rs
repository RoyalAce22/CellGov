//! An access whose translation the cache holds, and that runs out of the
//! storage of its large page into the next page, ends the batch when a
//! buffered store writes a byte of the next page's search. The attempt
//! then publishes none of its own effects.

use crate::mmu::Partition;
use crate::supervisor::{SlbEntry, SupervisorState};
use crate::*;
use cellgov_effects::Effect;
use cellgov_event::UnitId;
use cellgov_exec::{ExecutionContext, ExecutionStepResult, ExecutionUnit, YieldReason};
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_ps3_abi::hw::ppu_mmu::{PPE_PTE_LP, PTE_C, PTE_L, PTE_R, PTE_R_BYTE, SLB_L};
use cellgov_ps3_abi::hw::ppu_supervisor::{MSR_DR, MSR_IR, MSR_ME, MSR_SF, PPE_SLB_LP, SLB_VALID};
use cellgov_time::Budget;

/// The page table: 2^11 groups from 256 KB to 512 KB.
const HTAB: u64 = 0x4_0000;
/// The segment of 4 KB pages that holds the code, and its VSID.
const SEGMENT: u64 = 0xC000_0000;
const VSID: u64 = 0x123;
/// The real address of page 0 of the segment, which holds the code.
const CODE_RA: u64 = 0x3000;
/// The segment of 64 KB pages (L = 1, LP = 1), and its VSID.
const LARGE_EA: u64 = 0xD000_0000;
const LARGE_VSID: u64 = 0x456;
const LARGE_PAGE: u64 = 0x1_0000;
/// The real address of large page 0. The storage ends 16 bytes before
/// the end of that page.
const LARGE_RA: u64 = 0x8_0000;
const STORAGE_END: u64 = LARGE_RA + LARGE_PAGE - 16;
/// The real address of large page 1.
const NEXT_RA: u64 = 0x2_0000;
/// Bytes in a group of page table entries.
const GROUP: u64 = 128;

fn lwz(rt: u32, d: u16, ra: u32) -> u32 {
    (32 << 26) | (rt << 21) | (ra << 16) | u32::from(d)
}

fn stb(rs: u32, d: u16, ra: u32) -> u32 {
    (38 << 26) | (rs << 21) | (ra << 16) | u32::from(d)
}

fn lmw(rt: u32, d: u16, ra: u32) -> u32 {
    (46 << 26) | (rt << 21) | (ra << 16) | u32::from(d)
}

fn put(mem: &mut GuestMemory, at: u64, bytes: &[u8]) {
    let range = ByteRange::new(GuestAddr::new(at), bytes.len() as u64).unwrap();
    mem.apply_commit(range, bytes).unwrap();
}

/// The real address of slot 0 of the primary group for the page whose
/// hash is `hash`.
fn entry(hash: u64) -> u64 {
    HTAB + (hash & 0x7FF) * GROUP
}

/// Writes the entry at `at` with VSID `vsid`, the L bit `large`, and
/// doubleword 1 `d1`.
fn map(mem: &mut GuestMemory, at: u64, vsid: u64, large: bool, d1: u64) {
    let d0 = (vsid << 12) | if large { PTE_L } else { 0 } | 1;
    put(mem, at, &d0.to_be_bytes());
    put(mem, at + 8, &d1.to_be_bytes());
}

/// Doubleword 1 of the entry of large page 1: read/write, R = 0.
const NEXT_D1: u64 = NEXT_RA | PPE_PTE_LP | 0b10;

/// The real address of the entry of large page 1. The primary hash of a
/// 64 KB page XORs the VSID with the page's index in the segment.
fn next_entry() -> u64 {
    entry(LARGE_VSID ^ 1)
}

/// The storage, which ends at [`STORAGE_END`], with these entries:
/// - page 0 of the segment over the code, with R and C set;
/// - page 3 of the segment over the real page that holds the entry of
///   large page 1, with R and C set;
/// - large page 0 at [`LARGE_RA`], with R and C set;
/// - large page 1 at [`NEXT_RA`], with R = 0.
fn memory(code: &[u32]) -> GuestMemory {
    let mut mem = GuestMemory::new(STORAGE_END as usize);
    let rc = PTE_R | PTE_C | 0b10;
    map(&mut mem, entry(VSID), VSID, false, CODE_RA | rc);
    let entries_page = next_entry() & !0xFFF;
    map(&mut mem, entry(VSID ^ 3), VSID, false, entries_page | rc);
    map(
        &mut mem,
        entry(LARGE_VSID),
        LARGE_VSID,
        true,
        LARGE_RA | PPE_PTE_LP | rc,
    );
    map(&mut mem, next_entry(), LARGE_VSID, true, NEXT_D1);
    for (i, word) in code.iter().enumerate() {
        put(&mut mem, CODE_RA + 4 * i as u64, &word.to_be_bytes());
    }
    mem
}

/// A unit at the segment's first word, with relocation on, MSR\[ME\] = 1,
/// SLB entry 0 over the segment, SLB entry 1 over the large segment, and
/// the page table at [`HTAB`].
fn kernel_unit() -> PpuExecutionUnit {
    let mut sup = SupervisorState::new(MSR_SF | MSR_IR | MSR_DR | MSR_ME);
    sup.slb[0] = SlbEntry {
        esid_v: SEGMENT | SLB_VALID,
        vsid_flags: VSID << 12,
    };
    sup.slb[1] = SlbEntry {
        esid_v: LARGE_EA | SLB_VALID,
        vsid_flags: (LARGE_VSID << 12) | SLB_L | PPE_SLB_LP,
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

/// The read intents in large page 0, as start and length, in order.
fn large_page_reads(effects: &[Effect]) -> Vec<(u64, u64)> {
    let page = LARGE_RA..LARGE_RA + LARGE_PAGE;
    effects
        .iter()
        .filter_map(|e| match e {
            Effect::SharedReadIntent { range, .. } if page.contains(&range.start().raw()) => {
                Some((range.start().raw(), range.length()))
            }
            _ => None,
        })
        .collect()
}

/// Every write intent, as start and bytes, in order.
fn writes(effects: &[Effect]) -> Vec<(u64, Vec<u8>)> {
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

/// [CBE-Handbook p:90 s:4.2.6.8] The PPE performs every store, its page table entry updates included, in order.
#[test]
fn a_cached_access_that_ends_the_batch_at_the_next_pages_search_publishes_none_of_its_reads() {
    // The lwz caches the translation of large page 0. The stb writes the
    // R byte of large page 1's entry through page 3, with its own value.
    // The lmw then loads twelve words: four in the storage of large page
    // 0, four past the storage's end, and four in large page 1.
    let r_byte = NEXT_D1.to_be_bytes()[PTE_R_BYTE - 8];
    let r_byte_at = next_entry() + PTE_R_BYTE as u64;
    let lmw_ea = LARGE_EA + LARGE_PAGE - 32;
    let mem = memory(&[lwz(4, 0, 5), stb(6, 0, 7), lmw(20, 0, 8)]);
    let mut unit = kernel_unit();
    unit.state_mut().set_gpr(5, LARGE_EA);
    unit.state_mut().set_gpr(6, u64::from(r_byte));
    unit.state_mut()
        .set_gpr(7, SEGMENT + 0x3000 + (r_byte_at & 0xFFF));
    unit.state_mut().set_gpr(8, lmw_ea);
    let (r, effects) = run(&mut unit, &mem, 3);
    assert_eq!(r.yield_reason, YieldReason::BudgetExhausted);
    assert_eq!(unit.retired_instructions(), 2, "the lmw waits for the stb");
    assert_eq!(unit.state().pc, SEGMENT + 8);
    assert_eq!(
        large_page_reads(&effects),
        vec![(LARGE_RA, 4)],
        "only the lwz reads large page 0"
    );
    assert_eq!(
        writes(&effects),
        vec![(r_byte_at, vec![r_byte])],
        "only the stb"
    );
}
