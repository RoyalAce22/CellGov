use super::*;
use crate::hw::ppu_supervisor::{PPE_SLB_LP, SLB_VSID_FLAGS};

#[test]
fn the_slb_flags_sit_in_the_virtual_half_after_the_vsid() {
    assert_eq!([SLB_KS, SLB_KP, SLB_N, SLB_L], [0x800, 0x400, 0x200, 0x100]);
    for flag in [SLB_KS, SLB_KP, SLB_N, SLB_L] {
        assert_ne!(
            flag & SLB_VSID_FLAGS,
            0,
            "{flag:#x} is not an RS bit slbmte loads"
        );
        assert_eq!(flag & PPE_SLB_LP, 0);
    }
}

#[test]
fn the_pte_doubleword_0_fields_tile_without_overlap() {
    assert_eq!([PTE_V, PTE_H, PTE_L], [0x1, 0x2, 0x4]);
    assert_eq!(PPE_PTE_AVPN & (PTE_V | PTE_H | PTE_L), 0);
    // Bits 15:56: the AVPN field (0:56) less the PPE's reserved 0:14.
    assert_eq!(PPE_PTE_AVPN, (!0u64 >> 15) & (!0u64 << 7));
}

#[test]
fn the_pte_doubleword_1_fields_tile_without_overlap() {
    assert_eq!(PTE_RPN, (!0u64 >> 2) & (!0u64 << 12));
    // Bits 22:51: the RPN less the bits past a 42-bit real address.
    assert_eq!(PPE_PTE_RPN, PTE_RPN & ((1u64 << PPE_REAL_ADDRESS_BITS) - 1));
    assert_eq!(PPE_PTE_RPN.count_ones(), 30);
    assert_eq!([PTE_I, PTE_G, PTE_N, PTE_PP], [0x20, 0x8, 0x4, 0x3]);
    assert_eq!(PPE_PTE_LP, 0x1000);
    assert_eq!(PPE_PTE_LP & PTE_RPN, PPE_PTE_LP, "LP is the RPN's low bit");
    for field in [PTE_I, PTE_G, PTE_N, PTE_PP] {
        assert_eq!(field & PTE_RPN, 0);
    }
}

#[test]
fn r_and_c_sit_in_adjacent_bytes_of_doubleword_1() {
    assert_eq!([PTE_R, PTE_C], [0x100, 0x80]);
    let in_entry = |byte: usize, mask: u8| u64::from(mask) << (8 * (PTE_BYTES - 1 - byte));
    assert_eq!(in_entry(PTE_R_BYTE, PTE_R_BYTE_MASK), PTE_R);
    assert_eq!(in_entry(PTE_C_BYTE, PTE_C_BYTE_MASK), PTE_C);
    for field in [PTE_RPN, PTE_I, PTE_G, PTE_N, PTE_PP] {
        assert_eq!(field & (PTE_R | PTE_C), 0);
    }
}

#[test]
fn sdr1_holds_a_page_table_origin_of_44_bits_and_a_size_of_5() {
    assert_eq!(SDR1_HTABORG.count_ones(), 44);
    assert_eq!(SDR1_HTABORG.trailing_zeros(), 18);
    assert_eq!(SDR1_HTABSIZE, 0x1F);
    assert_eq!(SDR1_HTABORG & SDR1_HTABSIZE, 0);
    // The smallest table, 2^11 groups of 128 bytes, is 256 KB: the
    // origin's granularity.
    assert_eq!(
        (1u64 << HASH_INDEX_MIN_BITS) * PTEG_BYTES as u64,
        1 << SDR1_HTABORG.trailing_zeros()
    );
    assert_eq!(PTES_PER_PTEG * PTE_BYTES, PTEG_BYTES);
    // The largest origin the PPE allows is the last 256 KB of its real
    // address space.
    assert_eq!(
        (PPE_HTABORG_MAX + 1) << SDR1_HTABORG.trailing_zeros(),
        1 << PPE_REAL_ADDRESS_BITS
    );
}

#[test]
fn hid6_lb_decodes_to_the_three_large_page_sizes_and_one_reserved() {
    assert_eq!(large_page_log2(0b10), Some(16));
    assert_eq!(large_page_log2(0b01), Some(20));
    assert_eq!(large_page_log2(0b00), Some(24));
    assert_eq!(large_page_log2(0b11), None);
    // Only the field's two bits count.
    assert_eq!(large_page_log2(0b110), Some(16));
}

#[test]
fn real_mode_ignores_the_two_high_effective_address_bits() {
    assert_eq!(REAL_MODE_IGNORED_EA, (1 << 63) | (1 << 62));
    assert_eq!(RMLR_LIMIT_LOG2, (12, 62));
    assert_eq!(1u64 << SEGMENT_LOG2, 256 << 20);
    assert_eq!(1u64 << SMALL_PAGE_LOG2, 4096);
}
