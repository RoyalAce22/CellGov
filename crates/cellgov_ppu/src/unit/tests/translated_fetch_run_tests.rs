//! A translating thread's fetch runs reach the effect list at real
//! addresses when its block branches between pages more times than the
//! unit tracks runs apart.

use crate::mmu::Partition;
use crate::supervisor::{SlbEntry, SupervisorState};
use crate::*;
use cellgov_effects::Effect;
use cellgov_event::UnitId;
use cellgov_exec::{ExecutionContext, ExecutionUnit, YieldReason};
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_ps3_abi::hw::ppu_supervisor::{MSR_DR, MSR_IR, MSR_SF, SLB_VALID};
use cellgov_time::Budget;

/// The page table: 2^11 groups at 256 KB.
const HTAB: u64 = 0x4_0000;
/// The segment the SLB maps: EA 0xC000_0000, VSID 0x123.
const SEGMENT: u64 = 0xC000_0000;
const VSID: u64 = 0x123;
/// Page 0 of the segment, at real 0x3000.
const LOW_RA: u64 = 0x3000;
/// Page 2 of the segment, at real 0x6000. Page 1 holds no code.
const HIGH_RA: u64 = 0x6000;

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

#[test]
fn a_block_that_branches_between_two_pages_past_the_run_limit_publishes_only_real_fetches() {
    // Page 0: b +0x2000. Page 2: b -0x2000. Each branch closes a run,
    // and the twelve runs span page 1, which the block never fetches.
    let mut mem = GuestMemory::new(0x8_0000);
    map_page(&mut mem, 0, LOW_RA);
    map_page(&mut mem, 2, HIGH_RA);
    put(&mut mem, LOW_RA, &0x4800_2000u32.to_be_bytes());
    put(&mut mem, HIGH_RA, &0x4BFF_E000u32.to_be_bytes());
    let mut sup = SupervisorState::new(MSR_SF | MSR_IR | MSR_DR);
    sup.slb[0] = SlbEntry {
        esid_v: SEGMENT | SLB_VALID,
        vsid_flags: VSID << 12,
    };
    sup.partition = Partition::new(HTAB, 1 << 62, 0, Partition::INITIAL.lb()).unwrap();
    let mut unit = PpuExecutionUnit::new(UnitId::new(0));
    unit.state_mut().set_supervisor(Some(sup));
    unit.state_mut().pc = SEGMENT;

    let mut effects = Vec::new();
    let r = unit.run_until_yield(Budget::new(12), &ExecutionContext::new(&mem), &mut effects);

    assert_eq!(
        r.yield_reason,
        YieldReason::BudgetExhausted,
        "{:?}",
        r.local_diagnostics
    );
    assert_eq!(unit.retired_instructions(), 12);
    // Every read except those of the page table groups the misses
    // searched.
    let reads: Vec<(u64, u64)> = effects
        .iter()
        .filter_map(|e| match e {
            Effect::SharedReadIntent { range, .. } => Some((range.start().raw(), range.length())),
            _ => None,
        })
        .filter(|&(start, _)| !(HTAB..HTAB + 0x4_0000).contains(&start))
        .collect();
    assert!(
        reads.iter().all(|&r| r == (LOW_RA, 4) || r == (HIGH_RA, 4)),
        "every fetch read is one real word the block ran: {reads:x?}"
    );
    assert!(reads.contains(&(LOW_RA, 4)), "{reads:x?}");
    assert!(reads.contains(&(HIGH_RA, 4)), "{reads:x?}");
}
