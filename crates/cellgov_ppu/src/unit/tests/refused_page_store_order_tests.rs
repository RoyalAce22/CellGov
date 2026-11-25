//! A misaligned store from a caching-inhibited page into a read-only
//! cacheable page takes one interrupt, whether a load cached the second
//! page's translation or the store meets that page cold. The C bit of the
//! read-only page decides which interrupt comes first.

use super::*;
use crate::exec::{ExecuteVerdict, PpuFault, PpuInterrupt};
use crate::mmu::Access;
use crate::store_buffer::StoreBuffer;
use crate::unit::translation::RefillStop;
use cellgov_exec::InterruptRecord;
use cellgov_ps3_abi::hw::ppu_interrupt::{DSISR_PROTECTION, DSISR_STORE};

fn lbz(rt: u32, d: u16, ra: u32) -> u32 {
    (34 << 26) | (rt << 21) | (ra << 16) | u32::from(d)
}

/// Doubleword 1 of page 4's entry: read/write and caching-inhibited with
/// R = 0 and C = 1, so the store writes no C there.
const INHIBITED_D1: u64 = PAGE_RA | PTE_I | PTE_C | 0b10;

/// The sth's EA: the last byte of page 4, so its halfword runs into page 5.
const CROSSING_EA: u64 = SEGMENT + (PAGE << 12) + 0xFFF;

/// The EA of page 5's first byte.
const NEXT_EA: u64 = SEGMENT + (NEXT << 12);

/// Doubleword 1 of page 5's entry: read-only and cacheable with R = 1 and
/// C = `c`.
fn read_only_cacheable(c: bool) -> u64 {
    NEXT_RA | PTE_R | if c { PTE_C } else { 0 } | 0b11
}

/// The memory and unit of `code` over page 4 with [`INHIBITED_D1`] and
/// page 5 with doubleword 1 `next`. r9 holds page 5's first byte.
fn arrangement(code: &[u32], next: u64) -> (GuestMemory, PpuExecutionUnit) {
    let mut mem = memory(code, INHIBITED_D1);
    map_page(&mut mem, NEXT, next);
    let mut unit = kernel_unit();
    unit.state_mut().set_gpr(9, NEXT_EA);
    (mem, unit)
}

/// Runs batches of `unit` until one takes an interrupt, at most four, and
/// commits each batch's writes to `mem`. Returns the interrupt and the
/// effects of each batch.
fn run_to_interrupt(
    unit: &mut PpuExecutionUnit,
    mem: &mut GuestMemory,
) -> (InterruptRecord, Vec<Vec<Effect>>) {
    let mut batches = Vec::new();
    for _ in 0..4 {
        let (r, effects) = run(unit, mem, 16);
        commit(mem, &effects);
        batches.push(effects);
        if let Some(interrupt) = r.local_diagnostics.interrupt {
            return (interrupt, batches);
        }
    }
    panic!("no interrupt in {} batches", batches.len());
}

/// The interrupt that the crossing sth takes against the views of the
/// translations that the cache of `unit` holds.
fn interrupt_against_cache(unit: &PpuExecutionUnit, mem: &GuestMemory) -> PpuInterrupt {
    let insn = crate::decode::decode(sth(4, 0xFFF, 5)).unwrap();
    let views = unit.translated_views(mem);
    let mut state = unit.state().clone();
    let verdict = crate::exec::execute(
        &insn,
        &mut state,
        UnitId::new(0),
        &views,
        &mut Vec::new(),
        &mut StoreBuffer::new(),
    );
    match verdict {
        ExecuteVerdict::Fault(PpuFault::Interrupt(interrupt)) => interrupt,
        ExecuteVerdict::StoreFault(e) => PpuInterrupt::data_storage(&e, true).unwrap(),
        other => panic!("{other:?}"),
    }
}

/// [CBE-Handbook p:290 s:9.10] A store's alignment interrupt in caching-inhibited storage precedes the page-protection data storage interrupt when C is 1.
/// [CBE-Handbook p:90 s:4.2.6.8] Footnote 4: a misaligned caching-inhibited store is speculative, so its table lookup is not performed when it takes the alignment interrupt.
/// [PPC-Book3 p:70 s:5.5.8] An alignment interrupt sets DAR to the EA.
#[test]
fn a_misaligned_inhibited_store_into_a_read_only_cacheable_page_with_c_1_takes_the_alignment_interrupt_when_it_meets_that_page_cold(
) {
    let (mut mem, mut unit) = arrangement(&[sth(4, 0xFFF, 5)], read_only_cacheable(true));
    let (interrupt, batches) = run_to_interrupt(&mut unit, &mut mem);
    assert_eq!(batches.len(), 1, "the sth leads its batch");
    assert_eq!(interrupt.vector, irq::VECTOR_ALIGNMENT);
    assert_eq!(interrupt.dar, Some(CROSSING_EA));
    assert_r_byte_unchanged(&mem, PAGE, INHIBITED_D1);
}

/// [CBE-Handbook p:290 s:9.10] A store's alignment interrupt in caching-inhibited storage precedes the page-protection data storage interrupt when C is 1.
/// [CBE-Handbook p:90 s:4.2.6.8] Footnote 4: a misaligned caching-inhibited store is speculative, so its table lookup is not performed when it takes the alignment interrupt.
#[test]
fn a_misaligned_inhibited_store_after_a_load_of_its_read_only_cacheable_next_page_with_c_1_keeps_no_first_page_r_in_either_batch(
) {
    // The lbz caches page 5's load translation, so the sth meets page 5
    // through that translation. The fault ends the batch after the lbz,
    // and the sth leads the next batch, which meets page 5 through a
    // search.
    let (mut mem, mut unit) =
        arrangement(&[lbz(8, 0, 9), sth(4, 0xFFF, 5)], read_only_cacheable(true));
    let (interrupt, batches) = run_to_interrupt(&mut unit, &mut mem);
    assert_eq!(batches.len(), 2, "the fault ends the lbz's batch");
    assert_eq!(unit.retired_instructions(), 1, "the lbz retires");
    assert_eq!(interrupt.vector, irq::VECTOR_ALIGNMENT);
    assert_eq!(interrupt.dar, Some(CROSSING_EA));
    for (batch, effects) in batches.iter().enumerate() {
        assert_eq!(
            r_writes(effects, PAGE),
            Vec::<u8>::new(),
            "batch {}: page 4 R",
            batch + 1
        );
    }
}

/// [CBE-Handbook p:290 s:9.10] A store's alignment interrupt in caching-inhibited storage follows the page-protection data storage interrupt when C is 0, and precedes it when C is 1.
/// [PPC-Book3 p:67 s:5.5.3] A data storage interrupt sets DAR to a byte in the first aligned doubleword that the store attempted in the page that caused it.
#[test]
fn a_misaligned_inhibited_store_into_a_read_only_cacheable_page_takes_one_interrupt_whether_a_load_cached_that_page_or_not(
) {
    for c in [true, false] {
        let (mem, mut warm) = arrangement(&[], read_only_cacheable(c));
        warm.follow_translation();
        assert!(
            warm.refill(&mem, NEXT_EA, Access::Load).is_ok(),
            "page 5 load"
        );
        assert!(
            warm.refill_for(&mem, CROSSING_EA, Access::Store).is_ok(),
            "page 4 store"
        );
        let hit = interrupt_against_cache(&warm, &mem);

        let mut cold = kernel_unit();
        cold.state_mut().set_gpr(9, NEXT_EA);
        cold.follow_translation();
        assert!(
            cold.refill_for(&mem, CROSSING_EA, Access::Store).is_ok(),
            "page 4 store"
        );
        let miss = match cold.refill_for(&mem, NEXT_EA, Access::Store) {
            Ok(_) => interrupt_against_cache(&cold, &mem),
            Err(RefillStop::Fault(PpuFault::Interrupt(interrupt))) => interrupt,
            Err(_) => panic!("C = {c}: the search of page 5 stops"),
        };

        assert_eq!(hit, miss, "C = {c}");
        let expected = if c {
            PpuInterrupt::Alignment {
                ea: CROSSING_EA,
                store: true,
            }
        } else {
            PpuInterrupt::DataStorage {
                ea: NEXT_EA,
                store: true,
                protection: true,
            }
        };
        assert_eq!(miss, expected, "C = {c}");
    }
}

/// [CBE-Handbook p:90 s:4.2.6.8] Footnote 4: a misaligned caching-inhibited store is speculative, so its table lookup is not performed when it takes the alignment interrupt.
/// [CBE-Handbook p:290 s:9.10] A store's alignment interrupt in caching-inhibited storage precedes the page-protection data storage interrupt when C is 1.
#[test]
fn a_misaligned_store_whose_batch_ends_after_a_load_of_its_read_only_cacheable_next_page_with_c_1_keeps_no_first_page_r(
) {
    // Page 4 is caching-inhibited and writable with R = C = 0. The lbz
    // caches page 5's load translation. The stb writes byte 15 of page 4's
    // entry with its value unchanged. The C write of page 4 waits for the
    // stb, so the batch ends ahead of the sth.
    let page = PAGE_RA | PTE_I | 0b10;
    let mut mem = memory(&[lbz(8, 0, 9), stb(6, 0, 7), sth(4, 0xFFF, 5)], page);
    map_page(&mut mem, NEXT, read_only_cacheable(true));
    let mut unit = kernel_unit();
    unit.state_mut()
        .set_gpr(6, u64::from(page.to_be_bytes()[PTE_C_BYTE - 8]));
    unit.state_mut().set_gpr(7, entry_byte_ea(PAGE, PTE_C_BYTE));
    unit.state_mut().set_gpr(9, NEXT_EA);
    let (_, first) = run(&mut unit, &mem, 3);
    assert_eq!(unit.retired_instructions(), 2, "the sth waits");
    commit(&mut mem, &first);
    let (r, second) = run(&mut unit, &mem, 1);
    commit(&mut mem, &second);
    assert_eq!(
        r.local_diagnostics.interrupt.map(|i| i.vector),
        Some(irq::VECTOR_ALIGNMENT),
        "batch 2"
    );
    for (batch, effects) in [(1, &first), (2, &second)] {
        assert_eq!(
            r_writes(effects, PAGE),
            Vec::<u8>::new(),
            "batch {batch}: page 4 R"
        );
    }
    assert_r_byte_unchanged(&mem, PAGE, page);
}

/// [PPC-Book3 p:67 s:5.5.3] A data storage interrupt sets DSISR bit 4 when storage protection refuses the access, and bit 6 for a store.
#[test]
fn an_aligned_store_that_meets_a_read_only_cacheable_page_with_c_1_cold_takes_the_page_protection_interrupt(
) {
    let (mut mem, mut unit) = arrangement(&[sth(4, 0, 9)], read_only_cacheable(true));
    let (interrupt, batches) = run_to_interrupt(&mut unit, &mut mem);
    assert_eq!(batches.len(), 1, "the sth leads its batch");
    assert_eq!(interrupt.vector, irq::VECTOR_DATA_STORAGE);
    assert_eq!(interrupt.dsisr, Some(DSISR_PROTECTION | DSISR_STORE));
    assert_eq!(interrupt.dar, Some(NEXT_EA));
}
