//! A misaligned store runs from a read-only cacheable page into a
//! caching-inhibited page. It takes one interrupt, whether a load cached
//! the second page's translation or not. The C bit of the read-only page
//! decides whether the alignment interrupt comes before the
//! page-protection data storage interrupt.

use super::*;
use cellgov_exec::InterruptRecord;
use cellgov_ps3_abi::hw::ppu_interrupt::{DSISR_PROTECTION, DSISR_STORE};

fn lbz(rt: u32, d: u16, ra: u32) -> u32 {
    (34 << 26) | (rt << 21) | (ra << 16) | u32::from(d)
}

/// The sth's EA: the last byte of page 4, so its halfword runs into page 5.
const CROSSING_EA: u64 = SEGMENT + (PAGE << 12) + 0xFFF;

/// The EA of page 5's first byte.
const NEXT_EA: u64 = SEGMENT + (NEXT << 12);

/// Doubleword 1 of page 4's entry: read-only and cacheable with R = 0.
/// `c` sets C.
///
/// [PPC-Book3 p:45 s:4.10.1] PP 11 permits reads only, for either key.
fn read_only_cacheable(c: bool) -> u64 {
    PAGE_RA | if c { PTE_C } else { 0 } | 0b11
}

/// Runs batches of `code` over page 4 with doubleword 1 `page` until one
/// takes an interrupt, at most four, and commits each batch's writes. r9
/// holds page 5's first byte. Returns the interrupt, the effects of all
/// batches and the memory.
fn run_to_interrupt(code: &[u32], page: u64) -> (InterruptRecord, Vec<Effect>, GuestMemory) {
    run_to_interrupt_over(code, page, NEXT_D1)
}

/// [`run_to_interrupt`] with page 5's entry at doubleword 1 `next`.
fn run_to_interrupt_over(
    code: &[u32],
    page: u64,
    next: u64,
) -> (InterruptRecord, Vec<Effect>, GuestMemory) {
    let mut mem = memory(code, page);
    map_page(&mut mem, NEXT, next);
    let mut unit = kernel_unit();
    unit.state_mut().set_gpr(9, NEXT_EA);
    let mut all = Vec::new();
    for _ in 0..4 {
        let (r, effects) = run(&mut unit, &mem, 16);
        commit(&mut mem, &effects);
        all.extend(effects);
        if let Some(interrupt) = r.local_diagnostics.interrupt {
            return (interrupt, all, mem);
        }
    }
    panic!("no interrupt in four batches");
}

/// [CBE-Handbook p:290 s:9.10] A store's alignment interrupt in caching-inhibited storage precedes the page-protection data storage interrupt when C is 1.
/// [CBE-Handbook p:90 s:4.2.6.8] Footnote 4: a misaligned caching-inhibited store is speculative, so its table lookup is not performed when it takes the alignment interrupt.
/// [PPC-Book3 p:70 s:5.5.8] An alignment interrupt sets DAR to the EA.
#[test]
fn a_misaligned_store_from_a_read_only_page_with_c_1_into_an_uncached_inhibited_page_takes_the_alignment_interrupt_and_keeps_no_r(
) {
    let page = read_only_cacheable(true);
    let (interrupt, effects, mem) = run_to_interrupt(&[sth(4, 0xFFF, 5)], page);
    assert_eq!(interrupt.vector, irq::VECTOR_ALIGNMENT);
    assert_eq!(interrupt.dar, Some(CROSSING_EA));
    for index in [PAGE, NEXT] {
        assert_eq!(
            r_writes(&effects, index),
            Vec::<u8>::new(),
            "page {index} R"
        );
    }
    assert_r_byte_unchanged(&mem, PAGE, page);
    assert_r_byte_unchanged(&mem, NEXT, NEXT_D1);
}

/// [CBE-Handbook p:290 s:9.10] A store's alignment interrupt in caching-inhibited storage precedes the page-protection data storage interrupt when C is 1.
#[test]
fn a_misaligned_store_from_a_read_only_page_with_c_1_into_an_inhibited_page_takes_one_interrupt_whether_a_load_cached_that_page_or_not(
) {
    let page = read_only_cacheable(true);
    let (cold, _, _) = run_to_interrupt(&[sth(4, 0xFFF, 5)], page);
    let (cached, _, _) = run_to_interrupt(&[lbz(8, 0, 9), sth(4, 0xFFF, 5)], page);
    assert_eq!(cached.vector, irq::VECTOR_ALIGNMENT, "after the lbz");
    assert_eq!(cold.vector, cached.vector, "cold against cached");
    assert_eq!(cold.dar, cached.dar, "cold against cached");
}

/// Doubleword 1 of page 5's entry: read-only and caching-inhibited with
/// R = C = 0.
const READ_ONLY_INHIBITED_NEXT: u64 = NEXT_RA | PTE_I | 0b11;

/// [CBE-Handbook p:290 s:9.10] A store's alignment interrupt in caching-inhibited storage precedes the page-protection data storage interrupt when C is 1.
#[test]
fn a_misaligned_store_from_a_read_only_page_with_c_1_into_a_read_only_inhibited_page_with_c_0_takes_one_interrupt_whether_a_load_cached_that_page_or_not(
) {
    let page = read_only_cacheable(true);
    let next = READ_ONLY_INHIBITED_NEXT;
    let (cold, effects, mem) = run_to_interrupt_over(&[sth(4, 0xFFF, 5)], page, next);
    let (cached, _, _) = run_to_interrupt_over(&[lbz(8, 0, 9), sth(4, 0xFFF, 5)], page, next);
    assert_eq!(cached.vector, irq::VECTOR_ALIGNMENT, "after the lbz");
    assert_eq!(cold.vector, cached.vector, "cold against cached");
    assert_eq!(cold.dar, Some(CROSSING_EA), "cold");
    for index in [PAGE, NEXT] {
        assert_eq!(
            r_writes(&effects, index),
            Vec::<u8>::new(),
            "cold: page {index} R"
        );
    }
    assert_r_byte_unchanged(&mem, NEXT, next);
}

/// [CBE-Handbook p:90 s:4.2.6.8] Footnote 4: a misaligned caching-inhibited store is speculative, so its table lookup is not performed when it takes the alignment interrupt.
/// [CBE-Handbook p:290 s:9.10] A store's alignment interrupt in caching-inhibited storage precedes the page-protection data storage interrupt when C is 1.
#[test]
fn a_misaligned_store_from_a_read_only_page_with_c_1_into_a_read_only_inhibited_page_with_c_0_whose_search_waits_for_a_store_keeps_no_r(
) {
    // The stb writes byte 14 of page 5's entry with its value unchanged.
    // The search of page 5 waits for the stb, so the batch ends ahead of
    // the sth.
    let page = read_only_cacheable(true);
    let next = READ_ONLY_INHIBITED_NEXT;
    let mut mem = memory(&[stb(6, 0, 7), sth(4, 0xFFF, 5)], page);
    map_page(&mut mem, NEXT, next);
    let mut unit = kernel_unit();
    unit.state_mut()
        .set_gpr(6, u64::from(next.to_be_bytes()[PTE_R_BYTE - 8]));
    unit.state_mut().set_gpr(7, entry_byte_ea(NEXT, PTE_R_BYTE));
    run_to_alignment(&mut unit, &mut mem);
    assert_r_byte_unchanged(&mem, PAGE, page);
    assert_r_byte_unchanged(&mem, NEXT, next);
}

/// [CBE-Handbook p:290 s:9.10] A store's alignment interrupt in caching-inhibited storage follows the page-protection data storage interrupt when C is 0.
/// [PPC-Book3 p:67 s:5.5.3] A data storage interrupt sets DSISR bit 4 when storage protection refuses the access, and bit 6 for a store.
#[test]
fn a_misaligned_store_from_a_read_only_page_with_c_0_into_an_uncached_inhibited_page_takes_the_page_protection_interrupt(
) {
    let (interrupt, _, _) = run_to_interrupt(&[sth(4, 0xFFF, 5)], read_only_cacheable(false));
    assert_eq!(interrupt.vector, irq::VECTOR_DATA_STORAGE);
    assert_eq!(interrupt.dsisr, Some(DSISR_PROTECTION | DSISR_STORE));
    assert_eq!(interrupt.dar, Some(CROSSING_EA));
}

/// [CBE-Handbook p:253 s:9.5.8] The PPE takes the alignment interrupt for a load or store whose operand is unaligned and in caching-inhibited storage.
/// [PPC-Book3 p:67 s:5.5.3] A data storage interrupt sets DSISR bit 4 when storage protection refuses the access, and bit 6 for a store.
#[test]
fn a_misaligned_store_from_a_read_only_page_with_c_1_into_a_cacheable_page_takes_the_page_protection_interrupt_and_keeps_no_next_page_r(
) {
    let next = NEXT_RA | 0b10;
    let (interrupt, effects, mem) =
        run_to_interrupt_over(&[sth(4, 0xFFF, 5)], read_only_cacheable(true), next);
    assert_eq!(interrupt.vector, irq::VECTOR_DATA_STORAGE);
    assert_eq!(interrupt.dsisr, Some(DSISR_PROTECTION | DSISR_STORE));
    assert_eq!(interrupt.dar, Some(CROSSING_EA));
    assert_eq!(r_writes(&effects, NEXT), Vec::<u8>::new(), "page 5 R");
    assert_r_byte_unchanged(&mem, NEXT, next);
}

/// [PPC-Book3 p:45 s:4.10.1] PP 11 permits reads only, for either key.
/// [PPC-Book3 p:67 s:5.5.3] A data storage interrupt sets DSISR bit 4 when storage protection refuses the access, and bit 6 for a store.
#[test]
fn a_misaligned_store_from_a_read_only_page_with_c_1_into_a_read_only_cacheable_page_takes_the_page_protection_interrupt_and_keeps_no_next_page_r(
) {
    let next = NEXT_RA | 0b11;
    let (interrupt, effects, mem) =
        run_to_interrupt_over(&[sth(4, 0xFFF, 5)], read_only_cacheable(true), next);
    assert_eq!(interrupt.vector, irq::VECTOR_DATA_STORAGE);
    assert_eq!(interrupt.dsisr, Some(DSISR_PROTECTION | DSISR_STORE));
    assert_eq!(interrupt.dar, Some(CROSSING_EA));
    assert_eq!(r_writes(&effects, NEXT), Vec::<u8>::new(), "page 5 R");
    assert_r_byte_unchanged(&mem, NEXT, next);
}

/// [CBE-Handbook p:90 s:4.2.6.8] Footnote 4: a misaligned caching-inhibited store is speculative, so its table lookup is not performed when it takes the alignment interrupt.
/// [CBE-Handbook p:290 s:9.10] A store's alignment interrupt in caching-inhibited storage precedes the page-protection data storage interrupt when C is 1.
#[test]
fn a_misaligned_store_from_a_read_only_page_with_c_1_whose_next_page_search_waits_for_a_store_keeps_no_r(
) {
    // The stb writes byte 14 of page 5's entry with its value unchanged.
    // The search of page 5 waits for the stb, so the batch ends ahead of
    // the sth.
    let page = read_only_cacheable(true);
    let mut mem = memory(&[stb(6, 0, 7), sth(4, 0xFFF, 5)], page);
    let mut unit = kernel_unit();
    unit.state_mut()
        .set_gpr(6, u64::from(NEXT_D1.to_be_bytes()[PTE_R_BYTE - 8]));
    unit.state_mut().set_gpr(7, entry_byte_ea(NEXT, PTE_R_BYTE));
    run_to_alignment(&mut unit, &mut mem);
    assert_r_byte_unchanged(&mem, PAGE, page);
    assert_r_byte_unchanged(&mem, NEXT, NEXT_D1);
}
