//! A misaligned store that runs from a cacheable page with no access
//! into a caching-inhibited page that the translation cache does not hold.
//! The C bit of the first page decides whether the alignment interrupt
//! comes before the page-protection data storage interrupt.

use super::*;
use cellgov_ps3_abi::hw::ppu_interrupt::{DSISR_PROTECTION, DSISR_STORE};
use cellgov_ps3_abi::hw::ppu_mmu::SLB_KS;

/// The sth's EA: the last byte of page 4, so its halfword runs into page 5.
const CROSSING_EA: u64 = SEGMENT + (PAGE << 12) + 0xFFF;

/// Doubleword 1 of page 4's entry: cacheable, PP = 00 and R = 0, so key
/// 1 has no access. `c` sets C.
fn no_access_cacheable(c: bool) -> u64 {
    PAGE_RA | if c { PTE_C } else { 0 }
}

/// A unit as [`kernel_unit`] gives it, whose SLB entry has Ks = 1.
///
/// [PPC-Book3 p:45 s:4.10.1] A thread in privileged state takes its key from Ks, and with key 1 PP 00 permits no access and PP 10 permits reads and writes.
fn key_one_unit() -> PpuExecutionUnit {
    let mut unit = kernel_unit();
    let mut sup = unit.state().supervisor().unwrap().clone();
    sup.slb[0].vsid_flags |= SLB_KS;
    unit.state_mut().set_supervisor(Some(sup));
    unit
}

/// Runs one batch of `unit` and commits its writes to `mem`. Returns the
/// interrupt it takes and its effects.
fn run_once(
    unit: &mut PpuExecutionUnit,
    mem: &mut GuestMemory,
) -> (cellgov_exec::InterruptRecord, Vec<Effect>) {
    let (r, effects) = run(unit, mem, 16);
    commit(mem, &effects);
    let interrupt = r.local_diagnostics.interrupt.expect("an interrupt");
    (interrupt, effects)
}

/// [CBE-Handbook p:290 s:9.10] A store's alignment interrupt in caching-inhibited storage precedes the page-protection data storage interrupt when C is 1.
/// [CBE-Handbook p:90 s:4.2.6.8] Footnote 4: a misaligned caching-inhibited store is speculative, so its table lookup is not performed when it takes the alignment interrupt.
/// [PPC-Book3 p:70 s:5.5.8] An alignment interrupt sets DAR to the EA.
#[test]
fn a_misaligned_store_from_a_no_access_page_with_c_1_into_an_uncached_inhibited_page_takes_the_alignment_interrupt_and_keeps_no_r(
) {
    let page = no_access_cacheable(true);
    let mut mem = memory(&[sth(4, 0xFFF, 5)], page);
    let mut unit = key_one_unit();
    let (interrupt, effects) = run_once(&mut unit, &mut mem);
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

/// [PPC-Book3 p:45 s:4.10.1] With key 1, PP 11 permits reads only.
/// [CBE-Handbook p:290 s:9.10] A store's alignment interrupt in caching-inhibited storage precedes the page-protection data storage interrupt when C is 1.
/// [PPC-Book3 p:70 s:5.5.8] An alignment interrupt sets DAR to the EA.
#[test]
fn a_misaligned_store_from_a_no_access_page_with_c_1_into_a_read_only_inhibited_page_with_c_0_takes_the_alignment_interrupt_whether_a_load_cached_that_page_or_not(
) {
    let lbz = |rt: u32, d: u16, ra: u32| (34 << 26) | (rt << 21) | (ra << 16) | u32::from(d);
    let next = NEXT_RA | PTE_I | 0b11;
    let run_code = |code: &[u32]| {
        let mut mem = memory(code, no_access_cacheable(true));
        map_page(&mut mem, NEXT, next);
        let mut unit = key_one_unit();
        unit.state_mut().set_gpr(9, SEGMENT + (NEXT << 12));
        // The lbz's refill can end its batch ahead of the sth.
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
    };
    let (cold, effects, mem) = run_code(&[sth(4, 0xFFF, 5)]);
    let (cached, _, _) = run_code(&[lbz(8, 0, 9), sth(4, 0xFFF, 5)]);
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

/// [CBE-Handbook p:290 s:9.10] A store's alignment interrupt in caching-inhibited storage follows the page-protection data storage interrupt when C is 0.
/// [PPC-Book3 p:67 s:5.5.3] A data storage interrupt sets DSISR bit 4 when storage protection refuses the access, and bit 6 for a store.
/// [PPC-Book3 p:67 s:5.5.3] DAR holds the EA of a byte in the first aligned doubleword that the access attempted in the page that caused the interrupt.
#[test]
fn a_misaligned_store_from_a_no_access_page_with_c_0_into_an_uncached_inhibited_page_takes_the_page_protection_interrupt(
) {
    let mut mem = memory(&[sth(4, 0xFFF, 5)], no_access_cacheable(false));
    let mut unit = key_one_unit();
    let (interrupt, _) = run_once(&mut unit, &mut mem);
    assert_eq!(interrupt.vector, irq::VECTOR_DATA_STORAGE);
    assert_eq!(interrupt.dsisr, Some(DSISR_PROTECTION | DSISR_STORE));
    assert_eq!(interrupt.dar, Some(CROSSING_EA));
}

/// [CBE-Handbook p:253 s:9.5.8] The PPE takes the alignment interrupt for a load or store whose operand is unaligned and in caching-inhibited storage.
/// [PPC-Book3 p:67 s:5.5.3] A data storage interrupt sets DSISR bit 4 when storage protection refuses the access, and bit 6 for a store.
/// [PPC-Book3 p:67 s:5.5.3] DAR holds the EA of a byte in the first aligned doubleword that the access attempted in the page that caused the interrupt.
#[test]
fn a_misaligned_store_from_a_no_access_page_with_c_1_into_a_cacheable_page_takes_the_page_protection_interrupt_and_keeps_no_next_page_r(
) {
    let next = NEXT_RA | 0b10;
    let mut mem = memory(&[sth(4, 0xFFF, 5)], no_access_cacheable(true));
    map_page(&mut mem, NEXT, next);
    let mut unit = key_one_unit();
    let (interrupt, effects) = run_once(&mut unit, &mut mem);
    assert_eq!(interrupt.vector, irq::VECTOR_DATA_STORAGE);
    assert_eq!(interrupt.dsisr, Some(DSISR_PROTECTION | DSISR_STORE));
    assert_eq!(interrupt.dar, Some(CROSSING_EA));
    assert_eq!(r_writes(&effects, NEXT), Vec::<u8>::new(), "page 5 R");
    assert_r_byte_unchanged(&mem, NEXT, next);
}

/// [CBE-Handbook p:90 s:4.2.6.8] Footnote 4: a misaligned caching-inhibited store is speculative, so its table lookup is not performed when it takes the alignment interrupt.
/// [CBE-Handbook p:290 s:9.10] A store's alignment interrupt in caching-inhibited storage precedes the page-protection data storage interrupt when C is 1.
#[test]
fn a_misaligned_store_from_a_no_access_page_with_c_1_whose_next_page_search_waits_for_a_store_keeps_no_r(
) {
    // The stb writes byte 14 of page 5's entry with its value unchanged.
    // The search of page 5 waits for the stb, so the batch ends ahead of
    // the sth.
    let page = no_access_cacheable(true);
    let mut mem = memory(&[stb(6, 0, 7), sth(4, 0xFFF, 5)], page);
    let mut unit = key_one_unit();
    unit.state_mut()
        .set_gpr(6, u64::from(NEXT_D1.to_be_bytes()[PTE_R_BYTE - 8]));
    unit.state_mut().set_gpr(7, entry_byte_ea(NEXT, PTE_R_BYTE));
    run_to_alignment(&mut unit, &mut mem);
    assert_r_byte_unchanged(&mem, PAGE, page);
    assert_r_byte_unchanged(&mem, NEXT, NEXT_D1);
}
