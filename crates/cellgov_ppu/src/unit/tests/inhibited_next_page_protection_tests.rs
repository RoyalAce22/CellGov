//! A misaligned store that runs from a cacheable page that refuses it with
//! C = 1 into a caching-inhibited page with C = 0 that refuses it too. The
//! protection of the second page does not change the interrupt.

use super::*;
use cellgov_ps3_abi::hw::ppu_mmu::SLB_KS;

/// The sth's EA: the last byte of page 4, so its halfword runs into page 5.
const CROSSING_EA: u64 = SEGMENT + (PAGE << 12) + 0xFFF;

/// Doubleword 1 of page 4's entry for each first page, with its name: PP
/// 00 or PP 11, cacheable with R = 0 and C = 1.
///
/// [PPC-Book3 p:45 s:4.10.1] With key 1, PP 00 permits no access and PP 11 permits reads only.
const FIRST_PAGES: [(&str, u64); 2] = [
    ("no access", PAGE_RA | PTE_C),
    ("read-only", PAGE_RA | PTE_C | 0b11),
];

/// Doubleword 1 of page 5's entry with no access for key 1:
/// caching-inhibited, PP = 00 and R = C = 0.
const NO_ACCESS_INHIBITED_NEXT: u64 = NEXT_RA | PTE_I;

/// Doubleword 1 of page 5's entry, read-only for key 1: caching-inhibited,
/// PP = 11 and R = C = 0.
const READ_ONLY_INHIBITED_NEXT: u64 = NEXT_RA | PTE_I | 0b11;

/// A unit as [`kernel_unit`] gives it, whose SLB entry has Ks = 1.
///
/// [PPC-Book3 p:45 s:4.10.1] A thread in privileged state takes its key from Ks.
fn key_one_unit() -> PpuExecutionUnit {
    let mut unit = kernel_unit();
    let mut sup = unit.state().supervisor().unwrap().clone();
    sup.slb[0].vsid_flags |= SLB_KS;
    unit.state_mut().set_supervisor(Some(sup));
    unit
}

/// Runs one batch of the sth at the end of page 4 with key 1, over page 4
/// at doubleword 1 `page` and page 5 at doubleword 1 `next`. Commits the
/// batch's writes. Returns the interrupt, the effects and the memory.
fn run_sth(page: u64, next: u64) -> (cellgov_exec::InterruptRecord, Vec<Effect>, GuestMemory) {
    let mut mem = memory(&[sth(4, 0xFFF, 5)], page);
    map_page(&mut mem, NEXT, next);
    let mut unit = key_one_unit();
    let (r, effects) = run(&mut unit, &mem, 16);
    commit(&mut mem, &effects);
    let interrupt = r.local_diagnostics.interrupt.expect("an interrupt");
    (interrupt, effects, mem)
}

/// [CBE-Handbook p:290 s:9.10] A store's alignment interrupt in caching-inhibited storage precedes the page-protection data storage interrupt when C is 1, with no condition on PP.
/// [CBE-Handbook p:90 s:4.2.6.8] Footnote 4: a misaligned caching-inhibited store is speculative, so its table lookup is not performed when it takes the alignment interrupt.
/// [PPC-Book3 p:70 s:5.5.8] An alignment interrupt sets DAR to the EA.
#[test]
fn a_misaligned_store_from_a_refusing_page_with_c_1_into_a_refusing_inhibited_page_with_c_0_takes_the_alignment_interrupt_with_no_access_or_read_only(
) {
    for (first, page) in FIRST_PAGES {
        let (read_only, read_only_effects, read_only_mem) = run_sth(page, READ_ONLY_INHIBITED_NEXT);
        let (no_access, no_access_effects, no_access_mem) = run_sth(page, NO_ACCESS_INHIBITED_NEXT);
        assert_eq!(
            read_only.vector,
            irq::VECTOR_ALIGNMENT,
            "{first}: read-only"
        );
        assert_eq!(
            no_access.vector,
            irq::VECTOR_ALIGNMENT,
            "{first}: no access"
        );
        assert_eq!(read_only.dar, Some(CROSSING_EA), "{first}: read-only DAR");
        assert_eq!(no_access.dar, Some(CROSSING_EA), "{first}: no access DAR");
        let runs = [
            (
                "read-only",
                READ_ONLY_INHIBITED_NEXT,
                &read_only_effects,
                &read_only_mem,
            ),
            (
                "no access",
                NO_ACCESS_INHIBITED_NEXT,
                &no_access_effects,
                &no_access_mem,
            ),
        ];
        for (next_name, next, effects, mem) in runs {
            for index in [PAGE, NEXT] {
                assert_eq!(
                    r_writes(effects, index),
                    Vec::<u8>::new(),
                    "{first}, {next_name}: page {index} R"
                );
            }
            assert_r_byte_unchanged(mem, PAGE, page);
            assert_r_byte_unchanged(mem, NEXT, next);
        }
    }
}

/// [CBE-Handbook p:90 s:4.2.6.8] Footnote 4: a misaligned caching-inhibited store is speculative, so its table lookup is not performed when it takes the alignment interrupt.
/// [CBE-Handbook p:290 s:9.10] A store's alignment interrupt in caching-inhibited storage precedes the page-protection data storage interrupt when C is 1.
#[test]
fn a_misaligned_store_from_a_refusing_page_with_c_1_into_a_no_access_inhibited_page_with_c_0_whose_search_waits_for_a_store_keeps_no_r(
) {
    for (_, page) in FIRST_PAGES {
        // The stb writes byte 14 of page 5's entry with its value unchanged.
        // The search of page 5 waits for the stb, so the batch ends ahead
        // of the sth.
        let next = NO_ACCESS_INHIBITED_NEXT;
        let mut mem = memory(&[stb(6, 0, 7), sth(4, 0xFFF, 5)], page);
        map_page(&mut mem, NEXT, next);
        let mut unit = key_one_unit();
        unit.state_mut()
            .set_gpr(6, u64::from(next.to_be_bytes()[PTE_R_BYTE - 8]));
        unit.state_mut().set_gpr(7, entry_byte_ea(NEXT, PTE_R_BYTE));
        run_to_alignment(&mut unit, &mut mem);
        assert_r_byte_unchanged(&mem, PAGE, page);
        assert_r_byte_unchanged(&mem, NEXT, next);
    }
}
