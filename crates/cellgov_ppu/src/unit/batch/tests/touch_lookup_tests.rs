//! The table lookup of a touch on a translating thread: which line the
//! touch reloads, and which touch the PPE cancels.
//!
//! Each page is 4 KB, and each effective page lies on a real page at
//! another address. So a touch's effective line and real line differ.

use crate::instruction::encode::encode;
use crate::instruction::PpuInstruction as I;
use crate::mmu::Partition;
use crate::supervisor::{SlbEntry, SupervisorState};
use crate::PpuExecutionUnit;
use cellgov_effects::Effect;
use cellgov_event::UnitId;
use cellgov_exec::{ExecutionContext, ExecutionStepResult, ExecutionUnit};
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_ps3_abi::hw::ppu_mmu::{
    PTE_BYTES, PTE_C, PTE_G, PTE_I, PTE_R, PTE_R_BYTE, PTE_R_BYTE_MASK,
};
use cellgov_ps3_abi::hw::ppu_supervisor::{MSR_DR, MSR_IR, MSR_ME, MSR_SF, SLB_VALID};
use cellgov_time::Budget;

#[path = "console_touch_lookup_tests.rs"]
mod console;

/// The page table: 2^11 groups from 256 KB.
const HTAB: u64 = 0x4_0000;
/// The segment that SLB entry 0 maps, and its VSID.
const SEGMENT: u64 = 0xC000_0000;
const VSID: u64 = 0x123;
/// The real address of page 0, which holds the code.
const CODE_RA: u64 = 0x3000;
/// Page 4 of the segment, and its real address.
const PAGE: u64 = 4;
const PAGE_EA: u64 = SEGMENT + 0x4000;
const PAGE_RA: u64 = 0x6000;
/// The line each touch names, on page 4.
const TOUCH_EA: u64 = PAGE_EA + 0x80;
const TOUCH_RA: u64 = PAGE_RA + 0x80;
/// Page 5, which no test maps unless it names it.
const NEXT: u64 = 5;
const NEXT_EA: u64 = SEGMENT + 0x5000;
/// Page 6: the table holds no entry for it.
const UNMAPPED_EA: u64 = SEGMENT + 0x6080;
/// An address in a segment that no SLB entry maps.
const NO_SEGMENT_EA: u64 = 0xD000_0080;
/// Bytes in a group of page table entries.
const GROUP: u64 = 128;
/// Doubleword 1 of a read/write entry with R = C = 0 for real page `ra`.
const fn read_write(ra: u64) -> u64 {
    ra | 0b10
}

fn word(insn: I) -> u32 {
    encode(&insn).expect("the test instruction encodes")
}

/// `dcbt 0,r7`: a touch of the line at r7 with TH = 0.
fn dcbt_r7() -> u32 {
    word(I::Dcbt {
        ra: 0,
        rb: 7,
        th: 0,
    })
}

fn put(mem: &mut GuestMemory, at: u64, bytes: &[u8]) {
    let range = ByteRange::new(GuestAddr::new(at), bytes.len() as u64).unwrap();
    mem.apply_commit(range, bytes).unwrap();
}

/// The real address of the entry for page `index`: slot 0 of its primary
/// group, whose hash is VSID XOR the page index.
fn entry(index: u64) -> u64 {
    HTAB + ((VSID ^ index) & 0x7FF) * GROUP
}

fn map_page(mem: &mut GuestMemory, index: u64, d1: u64) {
    let d0 = (VSID << 12) | 1;
    put(mem, entry(index), &d0.to_be_bytes());
    put(mem, entry(index) + 8, &d1.to_be_bytes());
}

/// 512 KB that hold `code` on page 0 and an entry for each of `pages`,
/// as a page index and doubleword 1.
fn memory(code: &[u32], pages: &[(u64, u64)]) -> GuestMemory {
    let mut mem = GuestMemory::new(0x8_0000);
    map_page(&mut mem, 0, read_write(CODE_RA) | PTE_R | PTE_C);
    for &(index, d1) in pages {
        map_page(&mut mem, index, d1);
    }
    for (i, w) in code.iter().enumerate() {
        put(&mut mem, CODE_RA + 4 * i as u64, &w.to_be_bytes());
    }
    mem
}

/// A unit at the segment's first word, with relocation on, SLB entry 0
/// over the segment, the page table at [`HTAB`], r5 at [`PAGE_EA`] and r7
/// at `touched`.
fn kernel_unit(touched: u64) -> PpuExecutionUnit {
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
    unit.state_mut().set_gpr(7, touched);
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

/// The bytes of the one-byte writes to the R byte of page `index`'s
/// entry, in order.
fn reference_writes(effects: &[Effect], index: u64) -> Vec<u8> {
    let at = entry(index) + PTE_R_BYTE as u64;
    effects
        .iter()
        .filter_map(|e| match e {
            Effect::SharedWriteIntent { range, bytes, .. } if range.start().raw() == at => {
                Some(bytes.bytes()[0])
            }
            _ => None,
        })
        .collect()
}

/// The one R write that a search of an entry with doubleword 1 `d1` and
/// R = 0 makes: the entry's byte with R set.
fn r_set(d1: u64) -> Vec<u8> {
    vec![d1.to_be_bytes()[PTE_R_BYTE - PTE_BYTES / 2] | PTE_R_BYTE_MASK]
}

/// Asserts that the batch retired the touch at the segment's first word
/// and took no interrupt.
fn assert_retired_without_interrupt(unit: &PpuExecutionUnit, r: &ExecutionStepResult) {
    assert!(r.fault.is_none(), "the touch takes no fault: {:?}", r.fault);
    assert_eq!(unit.state().pc, SEGMENT + 4, "the touch retires");
}

/// Asserts that the timing model holds no line of `ea` and sent no touch
/// request.
fn assert_no_reload(unit: &PpuExecutionUnit, ea: u64, ra: u64) {
    let caches = unit.timing.caches();
    assert_eq!(caches.holds(ea, ea), (false, false), "no effective line");
    assert_eq!(caches.holds(ea, ra), (false, false), "no real line");
    assert_eq!(caches.touch_free(), 0, "no touch request");
}

/// [CBE-Handbook p:145 s:6.1.6.2] A touch that misses the TLB in hardware table-lookup mode reloads it and sets the R bit of the matching entry.
#[test]
fn an_untranslated_touch_reloads_its_translation_and_fills_the_real_line() {
    let mem = memory(&[dcbt_r7()], &[(PAGE, read_write(PAGE_RA))]);
    let mut unit = kernel_unit(TOUCH_EA);
    let (r, effects) = run(&mut unit, &mem, 1);
    assert_retired_without_interrupt(&unit, &r);
    let caches = unit.timing.caches();
    assert_eq!(caches.holds(TOUCH_EA, TOUCH_RA), (true, true));
    assert_eq!(caches.holds(TOUCH_EA, TOUCH_EA), (false, false));
    assert_eq!(reference_writes(&effects, PAGE), r_set(read_write(PAGE_RA)));
}

/// [CBE-Handbook p:145 s:6.1.6.2] A segment or page fault cancels the touch, and no exception occurs.
#[test]
fn a_touch_that_takes_a_segment_or_page_fault_is_cancelled_with_no_interrupt() {
    for touched in [UNMAPPED_EA, NO_SEGMENT_EA] {
        let mem = memory(&[dcbt_r7()], &[(PAGE, read_write(PAGE_RA))]);
        let mut unit = kernel_unit(touched);
        let (r, effects) = run(&mut unit, &mem, 1);
        assert_retired_without_interrupt(&unit, &r);
        assert_no_reload(&unit, touched, touched);
        assert!(reference_writes(&effects, PAGE).is_empty());
    }
}

/// [CBE-Handbook p:145 s:6.1.6.2] A touch of a caching-inhibited page is cancelled and does not reload the cache.
#[test]
fn a_touch_of_a_caching_inhibited_page_is_cancelled() {
    let d1 = read_write(PAGE_RA) | PTE_I;
    let mem = memory(&[dcbt_r7()], &[(PAGE, d1)]);
    let mut unit = kernel_unit(TOUCH_EA);
    let (r, effects) = run(&mut unit, &mem, 1);
    assert_retired_without_interrupt(&unit, &r);
    assert_no_reload(&unit, TOUCH_EA, TOUCH_RA);
    assert_eq!(reference_writes(&effects, PAGE), r_set(d1));
}

/// [CBE-Handbook p:145 s:6.1.6.2] A touch of a caching-inhibited page is cancelled and does not reload the cache.
///
/// No storage holds the page's real address, so no region view marks the
/// touch caching-inhibited, and only the table lookup can cancel it.
#[test]
fn a_touch_of_a_caching_inhibited_page_that_no_storage_holds_is_cancelled() {
    const UNBACKED_RA: u64 = 0x10_0000;
    let d1 = read_write(UNBACKED_RA) | PTE_I;
    let mem = memory(&[dcbt_r7()], &[(PAGE, d1)]);
    let mut unit = kernel_unit(TOUCH_EA);
    let (r, effects) = run(&mut unit, &mem, 1);
    assert_retired_without_interrupt(&unit, &r);
    assert_no_reload(&unit, TOUCH_EA, UNBACKED_RA + 0x80);
    assert_eq!(reference_writes(&effects, PAGE), r_set(d1));
}

/// [CBE-Handbook p:145 s:6.1.6.2] A touch of a guarded page is cancelled and does not reload the cache.
/// [CBE-Handbook p:90 s:4.2.6.7] Table 4-4: the PPE implements each cacheable WIMG setting as 0010, so a cacheable access is nonguarded.
#[test]
fn a_touch_of_a_cacheable_page_whose_entry_has_g_set_reloads_its_line() {
    let d1 = read_write(PAGE_RA) | PTE_G;
    let mem = memory(&[dcbt_r7()], &[(PAGE, d1)]);
    let mut unit = kernel_unit(TOUCH_EA);
    let (r, effects) = run(&mut unit, &mem, 1);
    assert_retired_without_interrupt(&unit, &r);
    let caches = unit.timing.caches();
    assert_eq!(caches.holds(TOUCH_EA, TOUCH_RA), (true, true));
    assert_eq!(reference_writes(&effects, PAGE), r_set(d1));
}

/// The entry of page 4 lies on page 7, which the store reaches. The
/// touch's R write would publish ahead of the buffered store, so the batch
/// ends ahead of the touch, as it ends ahead of a load miss there.
#[test]
fn a_touch_whose_r_write_meets_a_buffered_store_ends_the_batch_ahead_of_it() {
    const TABLE_PAGE: u64 = 7;
    let r_byte = entry(PAGE) + PTE_R_BYTE as u64;
    let stb = word(I::Stb {
        rs: 6,
        ra: 8,
        imm: 0,
    });
    let mem = memory(
        &[stb, dcbt_r7(), stb],
        &[
            (PAGE, read_write(PAGE_RA)),
            (TABLE_PAGE, read_write(r_byte & !0xFFF) | PTE_R | PTE_C),
        ],
    );
    let mut unit = kernel_unit(TOUCH_EA);
    unit.state_mut().set_gpr(6, 0x5A);
    unit.state_mut()
        .set_gpr(8, SEGMENT + (TABLE_PAGE << 12) + (r_byte & 0xFFF));
    let (r, effects) = run(&mut unit, &mem, 3);
    assert!(r.fault.is_none(), "no fault: {:?}", r.fault);
    assert_eq!(unit.state().pc, SEGMENT + 4, "the touch waits");
    assert_eq!(
        reference_writes(&effects, PAGE),
        vec![0x5A],
        "the store publishes, and the touch keeps no R write"
    );
}

/// A touch that waits for a buffered store retires nothing, so the next
/// batch stops at its break only after the skip the first batch spent.
#[test]
fn a_touch_that_waits_for_a_buffered_store_hands_back_its_break_skip() {
    const TABLE_PAGE: u64 = 7;
    let r_byte = entry(PAGE) + PTE_R_BYTE as u64;
    let stb = word(I::Stb {
        rs: 6,
        ra: 8,
        imm: 0,
    });
    let mem = memory(
        &[stb, dcbt_r7(), stb],
        &[
            (PAGE, read_write(PAGE_RA)),
            (TABLE_PAGE, read_write(r_byte & !0xFFF) | PTE_R | PTE_C),
        ],
    );
    let mut unit = kernel_unit(TOUCH_EA);
    unit.state_mut()
        .set_gpr(8, SEGMENT + (TABLE_PAGE << 12) + (r_byte & 0xFFF));
    unit.set_break_pc(SEGMENT + 4, 1);
    let (r, _) = run(&mut unit, &mem, 3);
    assert!(r.fault.is_none(), "no fault: {:?}", r.fault);
    assert_eq!(unit.state().pc, SEGMENT + 4, "the touch waits");
    let (r, _) = run(&mut unit, &mem, 1);
    assert!(
        r.fault.is_none(),
        "the touch passes its break: {:?}",
        r.fault
    );
    assert_eq!(unit.state().pc, SEGMENT + 8, "the touch retires");
}

/// The load translates page 4 first, from another line of the page.
#[test]
fn a_touch_of_a_page_the_batch_translated_fills_the_real_line() {
    let lwz = word(I::Lwz {
        rt: 6,
        ra: 5,
        imm: 0x800,
    });
    let mem = memory(&[lwz, dcbt_r7()], &[(PAGE, read_write(PAGE_RA))]);
    let mut unit = kernel_unit(TOUCH_EA);
    let (r, effects) = run(&mut unit, &mem, 2);
    assert!(r.fault.is_none(), "no fault: {:?}", r.fault);
    assert_eq!(unit.state().pc, SEGMENT + 8);
    let caches = unit.timing.caches();
    assert_eq!(caches.holds(TOUCH_EA, TOUCH_RA), (true, true));
    assert_ne!(caches.touch_free(), 0, "the touch sent its request");
    assert_eq!(reference_writes(&effects, PAGE), r_set(read_write(PAGE_RA)));
}

#[test]
fn a_load_after_a_touch_that_reloaded_its_translation_reads_the_page() {
    let lwz = word(I::Lwz {
        rt: 6,
        ra: 5,
        imm: 0x80,
    });
    let mut mem = memory(&[dcbt_r7(), lwz], &[(PAGE, read_write(PAGE_RA))]);
    put(&mut mem, TOUCH_RA, &0x1234_5678u32.to_be_bytes());
    let mut unit = kernel_unit(TOUCH_EA);
    let (r, _) = run(&mut unit, &mem, 2);
    assert!(r.fault.is_none(), "no fault: {:?}", r.fault);
    assert_eq!(unit.state().pc, SEGMENT + 8);
    assert_eq!(unit.state().gpr[6], 0x1234_5678);
}

/// Pages 4 and 5 both lie on [`PAGE_RA`]. A load through page 5 would not
/// see the store that the block buffers through page 4, so the batch ends
/// ahead of the touch, as it ends ahead of a load miss there.
#[test]
fn a_touch_whose_new_translation_aliases_a_buffered_store_ends_the_batch_ahead_of_it() {
    let stw = word(I::Stw {
        rs: 6,
        ra: 5,
        imm: 0,
    });
    let page = read_write(PAGE_RA) | PTE_R | PTE_C;
    let mem = memory(&[stw, dcbt_r7(), stw, stw], &[(PAGE, page), (NEXT, page)]);
    let mut unit = kernel_unit(NEXT_EA);
    let (r, effects) = run(&mut unit, &mem, 4);
    assert!(r.fault.is_none(), "no fault: {:?}", r.fault);
    assert_eq!(unit.state().pc, SEGMENT + 4, "the touch waits");
    let stores = effects
        .iter()
        .filter(|e| matches!(e, Effect::SharedWriteIntent { range, .. } if range.start().raw() == PAGE_RA))
        .count();
    assert_eq!(stores, 1, "the store publishes with the batch");
}
