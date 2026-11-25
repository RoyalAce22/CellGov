//! The R write of a table search and a later store to the same entry
//! byte reach storage in program order.

use super::*;

/// [CBE-Handbook p:90 s:4.2.6.8] The PPE performs all stores in order, its page table entry updates included.
#[test]
fn a_later_store_to_an_r_byte_lands_over_the_searchs_r_write() {
    // Page 3 maps the page of real memory that holds the entries of
    // pages 0, 1 and 3. Its own entry has R and C set, so a store through
    // it needs no entry write. The lwz writes R to byte 14 of page 1's
    // entry, and the stb then writes 0x54 to that byte.
    let page_of_entries = entry(1) & !0xFFF;
    let mut mem = memory(&[lwz(4, 0, 5), stb(6, 0, 7)]);
    map_page(&mut mem, 3, page_of_entries | PTE_R | PTE_C | 0b10);
    let mut unit = kernel_unit();
    unit.state_mut().set_gpr(5, DATA_EA);
    unit.state_mut().set_gpr(6, 0x54);
    unit.state_mut()
        .set_gpr(7, SEGMENT + 0x3000 + (entry(1) + 14 - page_of_entries));
    run_and_commit(&mut unit, &mut mem, 2);
    assert_eq!(
        unit.retired_instructions(),
        2,
        "the stb runs in the lwz's batch"
    );
    assert_eq!(entry_bytes(&mem, 1)[14], 0x54);
}
