//! A misaligned caching-inhibited store into a page that the thread may
//! neither load from nor store to orders its interrupts as it does on a
//! read-only page: the page's C bit decides whether the alignment
//! interrupt comes before the page-protection data storage interrupt.

use super::*;
use cellgov_ps3_abi::hw::ppu_interrupt::{DSISR_PROTECTION, DSISR_STORE};
use cellgov_ps3_abi::hw::ppu_mmu::SLB_KS;

/// The sth's EA: the last byte of page 4, so its halfword runs into page 5.
const CROSSING_EA: u64 = SEGMENT + (PAGE << 12) + 0xFFF;

/// The EA of page 5's first byte.
const NEXT_EA: u64 = SEGMENT + (NEXT << 12);

/// Doubleword 1 of page 4's entry: read/write for key 1 and
/// caching-inhibited with R = 0 and C = 1, so the store writes no C there.
const INHIBITED_D1: u64 = PAGE_RA | PTE_I | PTE_C | 0b10;

/// Doubleword 1 of an entry for real page `ra` with PP = 00 and R = 0:
/// no access for key 1. `extra` adds the I and C bits.
fn no_access(ra: u64, extra: u64) -> u64 {
    ra | extra
}

/// A unit as [`kernel_unit`] gives it, whose SLB entry has Ks = 1, with
/// r9 at page 5's first byte.
///
/// [PPC-Book3 p:45 s:4.10.1] A thread in privileged state takes its key from Ks, and with key 1 PP 00 permits no access and PP 10 permits reads and writes.
fn key_one_unit() -> PpuExecutionUnit {
    let mut unit = kernel_unit();
    let mut sup = unit.state().supervisor().unwrap().clone();
    sup.slb[0].vsid_flags |= SLB_KS;
    unit.state_mut().set_supervisor(Some(sup));
    unit.state_mut().set_gpr(9, NEXT_EA);
    unit
}

/// Runs one batch of `unit` and commits its writes to `mem`. Returns the
/// interrupt it takes and its effects.
fn run_once(
    unit: &mut PpuExecutionUnit,
    mem: &mut GuestMemory,
) -> (Option<cellgov_exec::InterruptRecord>, Vec<Effect>) {
    let (r, effects) = run(unit, mem, 16);
    commit(mem, &effects);
    (r.local_diagnostics.interrupt, effects)
}

/// [CBE-Handbook p:290 s:9.10] A store's alignment interrupt in caching-inhibited storage precedes the page-protection data storage interrupt when C is 1.
/// [CBE-Handbook p:90 s:4.2.6.8] Footnote 4: a misaligned caching-inhibited store is speculative, so its table lookup is not performed when it takes the alignment interrupt.
/// [PPC-Book3 p:70 s:5.5.8] An alignment interrupt sets DAR to the EA.
#[test]
fn a_misaligned_inhibited_store_into_a_page_with_no_access_and_c_1_takes_the_alignment_interrupt_and_keeps_no_r(
) {
    let next = no_access(NEXT_RA, PTE_C);
    let mut mem = memory(&[sth(4, 0xFFF, 5)], INHIBITED_D1);
    map_page(&mut mem, NEXT, next);
    let mut unit = key_one_unit();
    let (interrupt, effects) = run_once(&mut unit, &mut mem);
    let interrupt = interrupt.expect("the sth takes an interrupt");
    assert_eq!(interrupt.vector, irq::VECTOR_ALIGNMENT);
    assert_eq!(interrupt.dar, Some(CROSSING_EA));
    for index in [PAGE, NEXT] {
        assert_eq!(
            r_writes(&effects, index),
            Vec::<u8>::new(),
            "page {index} R"
        );
    }
    assert_r_byte_unchanged(&mem, PAGE, INHIBITED_D1);
    assert_r_byte_unchanged(&mem, NEXT, next);
}

/// [CBE-Handbook p:290 s:9.10] A store's alignment interrupt in caching-inhibited storage follows the page-protection data storage interrupt when C is 0.
/// [PPC-Book3 p:67 s:5.5.3] A data storage interrupt sets DSISR bit 4 when storage protection refuses the access, and bit 6 for a store.
#[test]
fn a_misaligned_inhibited_store_into_a_page_with_no_access_and_c_0_takes_the_page_protection_interrupt(
) {
    let mut mem = memory(&[sth(4, 0xFFF, 5)], INHIBITED_D1);
    map_page(&mut mem, NEXT, no_access(NEXT_RA, 0));
    let mut unit = key_one_unit();
    let (interrupt, _) = run_once(&mut unit, &mut mem);
    let interrupt = interrupt.expect("the sth takes an interrupt");
    assert_eq!(interrupt.vector, irq::VECTOR_DATA_STORAGE);
    assert_eq!(interrupt.dsisr, Some(DSISR_PROTECTION | DSISR_STORE));
    assert_eq!(interrupt.dar, Some(NEXT_EA));
}

/// [CBE-Handbook p:290 s:9.10] A store's alignment interrupt in caching-inhibited storage precedes the page-protection data storage interrupt when C is 1.
/// [PPC-Book3 p:70 s:5.5.8] An alignment interrupt sets DAR to the EA.
#[test]
fn a_misaligned_store_inside_an_inhibited_page_with_no_access_and_c_1_takes_the_alignment_interrupt(
) {
    let mut mem = memory(&[sth(4, 1, 9)], INHIBITED_D1);
    map_page(&mut mem, NEXT, no_access(NEXT_RA, PTE_I | PTE_C));
    let mut unit = key_one_unit();
    let (interrupt, _) = run_once(&mut unit, &mut mem);
    let interrupt = interrupt.expect("the sth takes an interrupt");
    assert_eq!(interrupt.vector, irq::VECTOR_ALIGNMENT);
    assert_eq!(interrupt.dar, Some(NEXT_EA + 1));
}

/// [CBE-Handbook p:253 s:9.5.8] The PPE takes the alignment interrupt for a load or store whose operand is unaligned and in caching-inhibited storage.
/// [PPC-Book3 p:67 s:5.5.3] A data storage interrupt sets DSISR bit 4 when storage protection refuses the access, and bit 6 for a store.
#[test]
fn an_aligned_store_into_an_inhibited_page_with_no_access_and_c_1_takes_the_page_protection_interrupt(
) {
    let mut mem = memory(&[sth(4, 0, 9)], INHIBITED_D1);
    map_page(&mut mem, NEXT, no_access(NEXT_RA, PTE_I | PTE_C));
    let mut unit = key_one_unit();
    let (interrupt, _) = run_once(&mut unit, &mut mem);
    let interrupt = interrupt.expect("the sth takes an interrupt");
    assert_eq!(interrupt.vector, irq::VECTOR_DATA_STORAGE);
    assert_eq!(interrupt.dsisr, Some(DSISR_PROTECTION | DSISR_STORE));
    assert_eq!(interrupt.dar, Some(NEXT_EA));
}

/// [CBE-Handbook p:290 s:9.10] A store's alignment interrupt in caching-inhibited storage precedes the page-protection data storage interrupt when C is 1.
/// [CBE-Handbook p:90 s:4.2.6.8] Footnote 4: a misaligned caching-inhibited store is speculative, so its table lookup is not performed when it takes the alignment interrupt.
#[test]
fn a_misaligned_store_from_an_inhibited_page_with_no_access_into_a_second_one_with_c_1_takes_the_alignment_interrupt(
) {
    let page = no_access(PAGE_RA, PTE_I | PTE_C);
    let next = no_access(NEXT_RA, PTE_C);
    let mut mem = memory(&[sth(4, 0xFFF, 5)], page);
    map_page(&mut mem, NEXT, next);
    let mut unit = key_one_unit();
    let (interrupt, effects) = run_once(&mut unit, &mut mem);
    let interrupt = interrupt.expect("the sth takes an interrupt");
    assert_eq!(interrupt.vector, irq::VECTOR_ALIGNMENT);
    assert_eq!(interrupt.dar, Some(CROSSING_EA));
    for index in [PAGE, NEXT] {
        assert_eq!(
            r_writes(&effects, index),
            Vec::<u8>::new(),
            "page {index} R"
        );
    }
}

/// [CBE-Handbook p:90 s:4.2.6.8] Footnote 4: a misaligned caching-inhibited store is speculative, so its table lookup is not performed when it takes the alignment interrupt.
/// [CBE-Handbook p:290 s:9.10] A store's alignment interrupt in caching-inhibited storage precedes the page-protection data storage interrupt when C is 1.
#[test]
fn a_misaligned_store_into_a_page_with_no_access_and_c_1_whose_batch_ends_ahead_of_it_keeps_no_first_page_r(
) {
    // Page 4 is caching-inhibited and writable with R = C = 0. The stb
    // writes byte 15 of page 4's entry with its value unchanged. The C
    // write of page 4 waits for the stb, so the batch ends ahead of the
    // sth.
    let page = PAGE_RA | PTE_I | 0b10;
    let mut mem = memory(&[stb(6, 0, 7), sth(4, 0xFFF, 5)], page);
    map_page(&mut mem, NEXT, no_access(NEXT_RA, PTE_C));
    let mut unit = key_one_unit();
    unit.state_mut()
        .set_gpr(6, u64::from(page.to_be_bytes()[PTE_C_BYTE - 8]));
    unit.state_mut().set_gpr(7, entry_byte_ea(PAGE, PTE_C_BYTE));
    let (_, first) = run(&mut unit, &mem, 2);
    assert_eq!(unit.retired_instructions(), 1, "the sth waits");
    commit(&mut mem, &first);
    let (interrupt, second) = run_once(&mut unit, &mut mem);
    assert_eq!(
        interrupt.map(|i| i.vector),
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
