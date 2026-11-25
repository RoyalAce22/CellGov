//! The prediction for a misaligned store whose batch ends ahead of it
//! sees the page table and the translations that the store's run in the
//! next batch sees: the table after this batch's stores commit, and each
//! page through a search.

use super::*;

fn lbz(rt: u32, d: u16, ra: u32) -> u32 {
    (34 << 26) | (rt << 21) | (ra << 16) | u32::from(d)
}

/// [CBE-Handbook p:90 s:4.2.6.8] Footnote 4: a misaligned caching-inhibited store is speculative, so its table lookup is not performed when it takes the alignment interrupt.
/// [CBE-Handbook p:253 s:9.5.8] The PPE takes the alignment interrupt for a load or store whose operand is unaligned and in caching-inhibited storage.
#[test]
fn a_misaligned_store_whose_batch_ends_at_a_buffered_store_that_sets_i_in_its_next_page_keeps_no_first_page_r(
) {
    // Page 5's entry is cacheable in memory. The first sth writes bytes
    // 14 and 15 of that entry and sets I. The search of page 5 waits for
    // that sth, so the batch ends ahead of the second sth.
    let page = PAGE_RA | PTE_C | 0b10;
    let mut mem = memory(&[sth(6, 0, 7), sth(4, 0xFFF, 5)], page);
    map_page(&mut mem, NEXT, NEXT_D1 & !PTE_I);
    let mut unit = kernel_unit();
    unit.state_mut().set_gpr(6, NEXT_D1 & 0xFFFF);
    unit.state_mut().set_gpr(7, entry_byte_ea(NEXT, PTE_R_BYTE));
    run_to_alignment(&mut unit, &mut mem);
    assert_r_byte_unchanged(&mem, PAGE, page);
    assert_r_byte_unchanged(&mem, NEXT, NEXT_D1);
}

/// [CBE-Handbook p:90 s:4.2.6.8] Each table lookup but that of a misaligned caching-inhibited store is nonspeculative, so it sets R whatever interrupt the access takes.
#[test]
fn a_misaligned_store_whose_batch_ends_after_a_load_of_its_read_only_cacheable_next_page_publishes_its_first_page_r(
) {
    // Page 4 is caching-inhibited and writable with R = C = 0. Page 5 is
    // read-only and cacheable with R = C = 1, and the lbz caches its load
    // translation. The stb writes byte 15 of page 4's entry with its
    // value unchanged. The C write of page 4 waits for the stb, so the
    // batch ends ahead of the sth. In the next batch the sth meets page 5
    // through a search, which refuses the store.
    let page = PAGE_RA | PTE_I | 0b10;
    let next = NEXT_RA | PTE_R | PTE_C | 0b11;
    let mut mem = memory(&[lbz(8, 0, 9), stb(6, 0, 7), sth(4, 0xFFF, 5)], page);
    map_page(&mut mem, NEXT, next);
    let mut unit = kernel_unit();
    unit.state_mut()
        .set_gpr(6, u64::from(page.to_be_bytes()[PTE_C_BYTE - 8]));
    unit.state_mut().set_gpr(7, entry_byte_ea(PAGE, PTE_C_BYTE));
    unit.state_mut().set_gpr(9, SEGMENT + (NEXT << 12));
    let (_, first) = run(&mut unit, &mem, 3);
    assert_eq!(unit.retired_instructions(), 2, "the sth waits");
    commit(&mut mem, &first);
    let (r, second) = run(&mut unit, &mem, 1);
    assert_eq!(
        r.local_diagnostics.interrupt.map(|i| i.vector),
        Some(irq::VECTOR_DATA_STORAGE),
        "batch 2"
    );
    let set = (page | PTE_R).to_be_bytes()[PTE_R_BYTE - 8];
    assert_eq!(r_writes(&first, PAGE), vec![set], "batch 1: page 4 R");
    assert_eq!(
        r_writes(&second, PAGE),
        Vec::<u8>::new(),
        "batch 2: page 4 R"
    );
}

/// [CBE-Handbook p:90 s:4.2.6.8] The PPE performs every store in order, its page table entry updates included.
/// [CBE-Handbook p:90 s:4.2.6.8] Footnote 4: a misaligned caching-inhibited store is speculative, so its table lookup is not performed when it takes the alignment interrupt.
#[test]
fn a_misaligned_store_whose_batch_ends_after_a_store_over_a_c_write_that_sets_i_keeps_no_first_page_r(
) {
    // Page 5 is cacheable and writable with R = C = 0. The first stb
    // writes page 5, so its searches write R and then C into page 5's
    // entry. The second stb writes byte 15 of that entry after the C
    // write and sets I with C = 0. The third stb writes byte 15 of page
    // 4's entry with its value unchanged, so the C write of page 4 waits
    // and the batch ends ahead of the sth. Page 5 is caching-inhibited
    // only when the second stb lands after the C write.
    let page = PAGE_RA | 0b10;
    let next = NEXT_RA | 0b10;
    let mut mem = memory(
        &[
            stb(10, 0, 9),
            stb(6, 0, 7),
            stb(11, 0, 12),
            sth(4, 0xFFF, 5),
        ],
        page,
    );
    map_page(&mut mem, NEXT, next);
    let mut unit = kernel_unit();
    unit.state_mut().set_gpr(9, SEGMENT + (NEXT << 12));
    unit.state_mut()
        .set_gpr(6, u64::from(NEXT_D1.to_be_bytes()[PTE_C_BYTE - 8]));
    unit.state_mut().set_gpr(7, entry_byte_ea(NEXT, PTE_C_BYTE));
    unit.state_mut()
        .set_gpr(11, u64::from(page.to_be_bytes()[PTE_C_BYTE - 8]));
    unit.state_mut()
        .set_gpr(12, entry_byte_ea(PAGE, PTE_C_BYTE));
    let (_, first) = run(&mut unit, &mem, 4);
    assert_eq!(unit.retired_instructions(), 3, "the sth waits");
    assert_eq!(
        entry_byte_writes(&first, NEXT, PTE_C_BYTE),
        vec![
            (next | PTE_C).to_be_bytes()[PTE_C_BYTE - 8],
            NEXT_D1.to_be_bytes()[PTE_C_BYTE - 8],
        ],
        "batch 1: page 5 C write, then the stb"
    );
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
