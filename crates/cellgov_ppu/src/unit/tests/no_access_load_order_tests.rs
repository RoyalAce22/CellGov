//! A misaligned load into caching-inhibited storage on a page that the
//! thread may neither load from nor store to takes the alignment interrupt
//! before the page-protection data storage interrupt, whatever the
//! page's C bit is.

use super::*;
use cellgov_ps3_abi::hw::ppu_interrupt::DSISR_PROTECTION;
use cellgov_ps3_abi::hw::ppu_mmu::SLB_KS;
use cellgov_ps3_abi::hw::ppu_supervisor::MSR_FP;

/// The lhz's EA: the last byte of page 4, so its halfword runs into page 5.
const CROSSING_EA: u64 = SEGMENT + (PAGE << 12) + 0xFFF;

/// The EA of page 5's first byte.
const NEXT_EA: u64 = SEGMENT + (NEXT << 12);

/// Doubleword 1 of an entry for real page `ra` with PP = 00 and R = 0:
/// no access for key 1. `extra` adds the I and C bits.
fn no_access(ra: u64, extra: u64) -> u64 {
    ra | extra
}

fn lhz(rt: u32, d: u16, ra: u32) -> u32 {
    (40 << 26) | (rt << 21) | (ra << 16) | u32::from(d)
}

fn lfd(frt: u32, d: u16, ra: u32) -> u32 {
    (50 << 26) | (frt << 21) | (ra << 16) | u32::from(d)
}

/// A unit as [`kernel_unit`] gives it, with:
/// - Ks = 1 in its SLB entry;
/// - MSR\[FP\] = 1;
/// - r9 at page 5's first byte.
///
/// [PPC-Book3 p:45 s:4.10.1] A thread in privileged state takes its key from Ks, and with key 1 PP 00 permits no access and PP 10 permits reads and writes.
fn key_one_unit() -> PpuExecutionUnit {
    let mut unit = kernel_unit();
    let mut sup = unit.state().supervisor().unwrap().clone();
    sup.slb[0].vsid_flags |= SLB_KS;
    sup.msr |= MSR_FP;
    unit.state_mut().set_supervisor(Some(sup));
    unit.state_mut().set_gpr(9, NEXT_EA);
    unit
}

/// Runs one batch of `code` against memory whose page 4 entry has
/// doubleword 1 `page` and whose page 5 entry has doubleword 1 `next`.
/// Returns the interrupt it takes and its effects.
fn run_load(code: &[u32], page: u64, next: u64) -> (cellgov_exec::InterruptRecord, Vec<Effect>) {
    let mut mem = memory(code, page);
    map_page(&mut mem, NEXT, next);
    let mut unit = key_one_unit();
    let (r, effects) = run(&mut unit, &mem, 16);
    let interrupt = r
        .local_diagnostics
        .interrupt
        .expect("the load takes an interrupt");
    (interrupt, effects)
}

/// Asserts that `effects` set R once in the entry of page 4 and once in
/// the entry of page 5.
fn assert_r_set_on_both_pages(effects: &[Effect]) {
    for index in [PAGE, NEXT] {
        assert_eq!(r_writes(effects, index).len(), 1, "page {index} R");
    }
}

/// [CBE-Handbook p:290 s:9.10] A fixed-point load's alignment interrupt in caching-inhibited storage precedes the page-protection data storage interrupt, with no condition on C.
/// [CBE-Handbook p:90 s:4.2.6.8] Each table lookup but that of a misaligned caching-inhibited store is nonspeculative, so it sets R whatever interrupt the access takes.
/// [PPC-Book3 p:70 s:5.5.8] An alignment interrupt sets DAR to the EA.
#[test]
fn a_misaligned_load_from_a_no_access_page_into_an_uncached_inhibited_page_takes_the_alignment_interrupt_and_sets_r(
) {
    let (interrupt, effects) = run_load(&[lhz(4, 0xFFF, 5)], no_access(PAGE_RA, 0), NEXT_D1);
    assert_eq!(interrupt.vector, irq::VECTOR_ALIGNMENT);
    assert_eq!(interrupt.dar, Some(CROSSING_EA));
    assert_r_set_on_both_pages(&effects);
}

/// [CBE-Handbook p:290 s:9.10] A fixed-point load's alignment interrupt in caching-inhibited storage precedes the page-protection data storage interrupt, with no condition on C.
/// [CBE-Handbook p:90 s:4.2.6.8] Each table lookup but that of a misaligned caching-inhibited store is nonspeculative, so it sets R whatever interrupt the access takes.
/// [PPC-Book3 p:70 s:5.5.8] An alignment interrupt sets DAR to the EA.
#[test]
fn a_misaligned_load_from_a_cacheable_page_into_a_no_access_inhibited_page_takes_the_alignment_interrupt_and_sets_r(
) {
    let (interrupt, effects) = run_load(
        &[lhz(4, 0xFFF, 5)],
        PAGE_RA | 0b10,
        no_access(NEXT_RA, PTE_I),
    );
    assert_eq!(interrupt.vector, irq::VECTOR_ALIGNMENT);
    assert_eq!(interrupt.dar, Some(CROSSING_EA));
    assert_r_set_on_both_pages(&effects);
}

/// [CBE-Handbook p:290 s:9.10] A fixed-point load's alignment interrupt in caching-inhibited storage precedes the page-protection data storage interrupt, with no condition on C.
/// [PPC-Book3 p:70 s:5.5.8] An alignment interrupt sets DAR to the EA.
#[test]
fn a_misaligned_load_from_an_inhibited_page_with_no_access_into_a_page_with_no_access_takes_the_alignment_interrupt(
) {
    let (interrupt, _) = run_load(
        &[lhz(4, 0xFFF, 5)],
        no_access(PAGE_RA, PTE_I),
        no_access(NEXT_RA, 0),
    );
    assert_eq!(interrupt.vector, irq::VECTOR_ALIGNMENT);
    assert_eq!(interrupt.dar, Some(CROSSING_EA));
}

/// [CBE-Handbook p:290 s:9.10] A fixed-point load's alignment interrupt in caching-inhibited storage precedes the page-protection data storage interrupt, with no condition on C.
/// [PPC-Book3 p:70 s:5.5.8] An alignment interrupt sets DAR to the EA.
#[test]
fn a_misaligned_load_from_a_cacheable_page_with_no_access_into_an_inhibited_page_with_no_access_takes_the_alignment_interrupt(
) {
    let (interrupt, _) = run_load(
        &[lhz(4, 0xFFF, 5)],
        no_access(PAGE_RA, 0),
        no_access(NEXT_RA, PTE_I),
    );
    assert_eq!(interrupt.vector, irq::VECTOR_ALIGNMENT);
    assert_eq!(interrupt.dar, Some(CROSSING_EA));
}

/// [CBE-Handbook p:290 s:9.10] A fixed-point load's alignment interrupt in caching-inhibited storage precedes the page-protection data storage interrupt, with no condition on C.
/// [PPC-Book3 p:70 s:5.5.8] An alignment interrupt sets DAR to the EA.
#[test]
fn a_misaligned_load_inside_an_inhibited_page_with_no_access_takes_the_alignment_interrupt_with_c_0_and_c_1(
) {
    for c in [0, PTE_C] {
        let (interrupt, _) = run_load(
            &[lhz(4, 1, 9)],
            PAGE_RA | 0b10,
            no_access(NEXT_RA, PTE_I | c),
        );
        assert_eq!(interrupt.vector, irq::VECTOR_ALIGNMENT, "C {c:#x}");
        assert_eq!(interrupt.dar, Some(NEXT_EA + 1), "C {c:#x}");
    }
}

/// [CBE-Handbook p:290 s:9.10] A word-aligned double-precision floating-point load that is not doubleword-aligned takes its alignment interrupt in caching-inhibited storage before the page-protection data storage interrupt.
/// [PPC-Book3 p:70 s:5.5.8] An alignment interrupt sets DAR to the EA.
#[test]
fn a_word_aligned_lfd_inside_an_inhibited_page_with_no_access_takes_the_alignment_interrupt() {
    let (interrupt, _) = run_load(&[lfd(1, 4, 9)], PAGE_RA | 0b10, no_access(NEXT_RA, PTE_I));
    assert_eq!(interrupt.vector, irq::VECTOR_ALIGNMENT);
    assert_eq!(interrupt.dar, Some(NEXT_EA + 4));
}

/// [CBE-Handbook p:253 s:9.5.8] The PPE takes the alignment interrupt for a load or store whose operand is unaligned and in caching-inhibited storage.
/// [PPC-Book3 p:67 s:5.5.3] A data storage interrupt sets DSISR bit 4 when storage protection refuses the access, and bit 6 for a store.
/// [PPC-Book3 p:67 s:5.5.3] DAR holds the EA of a byte in the first aligned doubleword that the access attempted in the page that caused the interrupt.
#[test]
fn an_aligned_load_inside_an_inhibited_page_with_no_access_takes_the_page_protection_interrupt() {
    let (interrupt, _) = run_load(
        &[lhz(4, 0, 9)],
        PAGE_RA | 0b10,
        no_access(NEXT_RA, PTE_I | PTE_C),
    );
    assert_eq!(interrupt.vector, irq::VECTOR_DATA_STORAGE);
    assert_eq!(interrupt.dsisr, Some(DSISR_PROTECTION));
    assert_eq!(interrupt.dar, Some(NEXT_EA));
}

/// [CBE-Handbook p:253 s:9.5.8] The PPE takes the alignment interrupt for a load or store whose operand is unaligned and in caching-inhibited storage.
/// [PPC-Book3 p:67 s:5.5.3] A data storage interrupt sets DSISR bit 4 when storage protection refuses the access, and bit 6 for a store.
/// [PPC-Book3 p:67 s:5.5.3] DAR holds the EA of a byte in the first aligned doubleword that the access attempted in the page that caused the interrupt.
#[test]
fn a_misaligned_load_into_a_cacheable_page_with_no_access_takes_the_page_protection_interrupt() {
    let cases = [(lhz(4, 1, 9), NEXT_EA + 1), (lhz(4, 0xFFF, 5), CROSSING_EA)];
    for (insn, dar) in cases {
        let (interrupt, _) = run_load(
            &[insn],
            no_access(PAGE_RA, PTE_C),
            no_access(NEXT_RA, PTE_C),
        );
        assert_eq!(interrupt.vector, irq::VECTOR_DATA_STORAGE, "{insn:#x}");
        assert_eq!(interrupt.dsisr, Some(DSISR_PROTECTION), "{insn:#x}");
        assert_eq!(interrupt.dar, Some(dar), "{insn:#x}");
    }
}

/// [CBE-Handbook p:90 s:4.2.6.8] Each table lookup but that of a misaligned caching-inhibited store is nonspeculative, so it sets R whatever interrupt the access takes.
/// [PPC-Book3 p:67 s:5.5.3] A data storage interrupt sets DSISR bit 4 when storage protection refuses the access.
/// [PPC-Book3 p:67 s:5.5.3] DAR holds the EA of a byte in the first aligned doubleword that the access attempted in the page that caused the interrupt.
#[test]
fn a_misaligned_load_from_a_cacheable_page_with_no_access_into_a_readable_cacheable_page_takes_the_page_protection_interrupt_and_sets_r_on_both(
) {
    let (interrupt, effects) = run_load(
        &[lhz(4, 0xFFF, 5)],
        no_access(PAGE_RA, PTE_C),
        NEXT_RA | 0b10,
    );
    assert_eq!(interrupt.vector, irq::VECTOR_DATA_STORAGE);
    assert_eq!(interrupt.dsisr, Some(DSISR_PROTECTION));
    assert_eq!(interrupt.dar, Some(CROSSING_EA));
    assert_r_set_on_both_pages(&effects);
}

/// [CBE-Handbook p:290 s:9.10] A fixed-point load's alignment interrupt in caching-inhibited storage precedes the page-protection data storage interrupt, with no condition on C.
/// [PPC-Book3 p:70 s:5.5.8] An alignment interrupt sets DAR to the EA.
#[test]
fn a_misaligned_load_from_an_unbacked_inhibited_page_into_a_page_with_no_access_takes_the_alignment_interrupt(
) {
    let (interrupt, _) = run_load(
        &[lhz(4, 0xFFF, 5)],
        UNBACKED_RA | PTE_I | 0b10,
        no_access(NEXT_RA, 0),
    );
    assert_eq!(interrupt.vector, irq::VECTOR_ALIGNMENT);
    assert_eq!(interrupt.dar, Some(CROSSING_EA));
}

/// [CBE-Handbook p:290 s:9.10] A load's page-fault data storage interrupt precedes its alignment interrupt in caching-inhibited storage, which precedes the page-protection data storage interrupt.
/// [PPC-Book3 p:67 s:5.5.3] A data storage interrupt sets DSISR bit 1 when the translation is not found in either group.
/// [PPC-Book3 p:67 s:5.5.3] DAR holds the EA of a byte in the first aligned doubleword that the access attempted in the page that caused the interrupt.
#[test]
fn a_misaligned_load_from_an_inhibited_page_with_no_access_into_a_page_with_no_entry_takes_the_page_fault(
) {
    let mut mem = memory(&[lhz(4, 0xFFF, 5)], no_access(PAGE_RA, PTE_I));
    // Page 5's slot gets doubleword 0 = 0, so V = 0 and no entry matches.
    put(&mut mem, entry(NEXT), &0u64.to_be_bytes());
    let mut unit = key_one_unit();
    let (r, _) = run(&mut unit, &mem, 16);
    let interrupt = r
        .local_diagnostics
        .interrupt
        .expect("the load takes an interrupt");
    assert_eq!(interrupt.vector, irq::VECTOR_DATA_STORAGE);
    assert_eq!(interrupt.dsisr, Some(irq::DSISR_NOT_FOUND));
    assert_eq!(interrupt.dar, Some(NEXT_EA));
}
