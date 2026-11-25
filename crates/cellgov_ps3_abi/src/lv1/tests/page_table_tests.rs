use super::*;
use crate::hw::ppu_mmu::{large_page_log2, PTE_C, PTE_G, PTE_I, PTE_N, PTE_PP, PTE_R};
use crate::lv1::status::{
    zero_extended, LV1_ALIGNMENT_ERROR, LV1_ILLEGAL_PARAMETER_VALUE, LV1_NO_ENTRY,
};
use crate::lv1::table::{table_entry, TableEntry};

/// The page shifts the 4.93 kernel passes to hypercall 2
/// [OFW-4.93 lv2_kernel a:0x800000000005dcc0-0x800000000005dce4]: 16 and
/// 20.
const KERNEL_PAGE_SHIFTS: u64 = 0x1014_0000_0000_0000;

fn marked(mark: u64, status: i64) -> u64 {
    (mark << 32) | zero_extended(status)
}

#[test]
fn the_4_93_handlers_are_the_live_table_entries_of_1_and_95() {
    let handlers = page_table_handlers("4.93").expect("4.93 handlers");
    assert_eq!(
        table_entry("4.93", 1),
        Some(TableEntry::Handler(handlers.write))
    );
    assert_eq!(
        table_entry("4.93", 95),
        Some(TableEntry::Handler(handlers.read))
    );
    assert_eq!(page_table_handlers("4.92"), None);
}

/// Hypercall 0 is `allocate_memory` in Linux `lv1call.h`.
#[test]
fn the_4_93_map_htab_and_allocate_memory_entries_share_the_memory_call_thunk() {
    assert_eq!(
        table_entry("4.93", 122),
        Some(TableEntry::Handler(0x2C89CC))
    );
    assert_eq!(table_entry("4.93", 0), Some(TableEntry::Handler(0x2C89CC)));
}

#[test]
fn each_status_marks_the_argument_its_check_reads() {
    let illegal = LV1_ILLEGAL_PARAMETER_VALUE;
    assert_eq!(HTAB_NO_ENTRY_R3, marked(0b0001, LV1_NO_ENTRY));
    assert_eq!(READ_HTAB_INDEX_R3, marked(0b0010, illegal));
    assert_eq!(WRITE_HTAB_INDEX_R3, marked(0b0010, illegal));
    assert_eq!(WRITE_HTAB_DOUBLEWORD0_R3, marked(0b0100, illegal));
    assert_eq!(WRITE_HTAB_DOUBLEWORD1_R3, marked(0b1000, illegal));
    assert_eq!(WRITE_HTAB_PAGE_SIZE_R3, marked(0b1000, LV1_NO_ENTRY));
    assert_eq!(WRITE_HTAB_ALIGNMENT_R3, marked(0b1000, LV1_ALIGNMENT_ERROR));
}

/// `LV1_DUPLICATE_ENTRY` is -7 and `LV1_RESOURCE_SHORTAGE` is -2 in Linux
/// `asm/ps3.h`.
#[test]
fn the_duplicate_and_map_htab_shortage_statuses_are_zero_extended() {
    assert_eq!(WRITE_HTAB_DUPLICATE_R3, 0x0000_0000_FFFF_FFF9);
    assert_eq!(MAP_HTAB_RESOURCE_SHORTAGE_R3, 0x0000_0000_FFFF_FFFE);
}

#[test]
fn the_doubleword_1_refusal_covers_both_reserved_fields() {
    let reserved = doubleword_bit(0) | doubleword_bit(1) | doubleword_bit(52) | doubleword_bit(53);
    assert_eq!(WRITE_HTAB_DOUBLEWORD1_RESERVED, reserved);
}

/// [CBE-Handbook p:86 s:4.2.6.2] Table 4-2: PTE bits 0:14 are reserved.
#[test]
fn the_doubleword_0_refusal_covers_bits_0_to_14() {
    let mut reserved = 0;
    for bit in 0..=14 {
        reserved |= doubleword_bit(bit);
    }
    assert_eq!(reserved.count_ones(), 15);
    assert_eq!(WRITE_HTAB_DOUBLEWORD0_RESERVED, reserved);
}

#[test]
fn a_page_table_of_two_to_the_eighteen_bytes_holds_16384_entries() {
    assert_eq!(page_table_entries(18), 16384);
    assert_eq!(page_table_entries(20), 65536);
}

#[test]
fn the_low_halfwords_pack_with_the_first_entry_most_significant() {
    let packed = read_htab_low_halfwords([
        0xAAAA_AAAA_AAAA_1111,
        0xBBBB_BBBB_BBBB_2222,
        0xCCCC_CCCC_CCCC_3333,
        0xDDDD_DDDD_DDDD_4444,
    ]);
    assert_eq!(packed, 0x1111_2222_3333_4444);
}

#[test]
fn the_large_page_index_counts_the_one_bits_from_bit_51_up() {
    assert_eq!(large_page_index(0x0000_0000_1234_0000), 0);
    assert_eq!(large_page_index(0x0000_0000_1234_1000), 1);
    assert_eq!(large_page_index(0x0000_0000_1234_B000), 2);
    assert_eq!(large_page_bits(0), 0);
    assert_eq!(large_page_bits(1), 0x1000);
    assert_eq!(large_page_bits(2), 0x3000);
}

#[test]
fn a_duplicate_ignores_the_software_bits_and_in_the_other_group_differs_in_h() {
    let new = 0x0000_1234_5678_9A01;
    assert!(duplicates(new | DUPLICATE_IGNORED, new, true));
    assert!(!duplicates(new ^ 0x80, new, true));
    assert!(!duplicates(new | PTE_H, new, true));
    assert!(duplicates(new | PTE_H, new, false));
    assert!(!duplicates(new, new, false));
}

#[test]
fn the_other_group_flips_every_group_index_bit() {
    assert_eq!(other_group(0, 18), 0x3FF8);
    assert_eq!(other_group(0x3FF8 + 3, 18), 0);
    assert_eq!(other_group(0x10 + 7, 18), 0x3FE8);
}

#[test]
fn the_stored_doubleword_1_takes_the_real_page_and_forces_wimg_to_m() {
    let written = 0x0000_0000_0123_4000 | PTE_R | PTE_C | 0x78 | PTE_N | PTE_PP;
    let stored = stored_doubleword1(written, 0x0040_0000, 0);
    assert_eq!(stored, 0x0040_0000 | PTE_R | PTE_C | PTE_M | PTE_N | PTE_PP);
    assert_eq!(stored & (PTE_I | PTE_G), 0);
    assert_eq!(stored_doubleword1(0x2, 0, 0x1000), 0x1000 | PTE_M | 0x2);
}

#[test]
fn sdr1_holds_the_table_address_and_the_exponent_less_18() {
    assert_eq!(select_sdr1(0x100_0000_0000, 18), 0x100_0000_0000);
    assert_eq!(select_sdr1(0x100_0000_0000, 20), 0x100_0000_0002);
}

#[test]
fn the_kernel_page_shifts_select_64k_pages_for_lp_0_and_1m_pages_for_lp_1() {
    let lb = select_hid6_lb(2, KERNEL_PAGE_SHIFTS);
    assert_eq!(lb, 0b1001);
    assert_eq!(large_page_log2(lb >> 2), Some(16));
    assert_eq!(large_page_log2(lb), Some(20));
}

#[test]
fn a_page_size_past_the_count_gives_lb_field_00() {
    assert_eq!(select_hid6_lb(0, KERNEL_PAGE_SHIFTS), 0);
    assert_eq!(select_hid6_lb(1, KERNEL_PAGE_SHIFTS), 0b1000);
}

#[test]
fn a_page_shift_of_24_gives_lb_field_00() {
    let lb = select_hid6_lb(2, 0x1810_0000_0000_0000);
    assert_eq!(lb, 0b0010);
    assert_eq!(large_page_log2(lb >> 2), Some(24));
    assert_eq!(large_page_log2(lb), Some(16));
}
