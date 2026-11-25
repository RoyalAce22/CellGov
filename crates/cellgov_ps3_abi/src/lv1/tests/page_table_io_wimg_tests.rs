//! The WIMG that lv1's `write_htab_entry` writer stores for a page above
//! [`STORED_MEMORY_LIMIT`].

use super::*;
use crate::hw::ppu_mmu::PTE_N;

/// Each real address and the WIMG for a caller with G clear, then with G
/// set, as the writer's branches give them. The addresses are literals
/// from the writer's compares [OFW-4.93 lv1 a:0x2D3178-0x2D31C4], so a
/// wrong limit constant turns a case red.
const CASES: [(u64, u64, u64); 7] = [
    (0x240_0301_0000, PTE_I | PTE_G, PTE_I | PTE_G),
    (0x280_8000_1000, PTE_I, PTE_I | PTE_G),
    (0x200_0000_0000, PTE_I, PTE_I | PTE_G),
    (0x200_0004_0000, PTE_I | PTE_G, PTE_I | PTE_G),
    (0x200_0008_0000, PTE_I, PTE_I | PTE_G),
    (0x1_0000_0000, PTE_I | PTE_G, PTE_I | PTE_G),
    (0x200_0040_0000, PTE_I | PTE_G, PTE_I | PTE_G),
];

#[test]
fn each_io_branch_stores_its_wimg_for_both_caller_g_bits() {
    assert_eq!(CASES.len(), 7);
    for (real, without_g, with_g) in CASES {
        assert_eq!(stored_io_wimg(0, real), without_g, "{real:#x} with G clear");
        assert_eq!(stored_io_wimg(PTE_G, real), with_g, "{real:#x} with G set");
    }
}

/// The writer compares bits 20:35 of the address, so the block holds
/// 256 MB from its base.
#[test]
fn the_caller_g_block_ends_256_mb_past_its_base() {
    assert_eq!(stored_io_wimg(0, 0x280_8FFF_F000), PTE_I);
    assert_eq!(stored_io_wimg(0, 0x280_9000_0000), PTE_I | PTE_G);
}

/// The writer clears the caller's W and M for a page above the memory
/// limit [OFW-4.93 lv1 a:0x2D31C8-0x2D31D4]. W is bit 57 of doubleword 1.
#[test]
fn an_io_entry_drops_the_callers_w_and_m() {
    let doubleword1 = doubleword_bit(57) | PTE_M | 0x2;
    assert_eq!(
        stored_io_doubleword1(doubleword1, 0x240_0301_0000, 0),
        0x240_0301_0000 | PTE_I | PTE_G | 0x2
    );
}

/// The 4.93 kernel's entry for the first page of its MMIO mapping keeps
/// N and PP, and gets I and G in place of its own WIMG.
#[test]
fn the_kernels_mmio_entry_stores_i_and_g_with_its_n_and_pp() {
    let doubleword1 = 0x0000_4000_0007_002C;
    assert_eq!(
        stored_io_doubleword1(doubleword1, 0x240_0301_0000, 0),
        0x240_0301_0000 | PTE_I | PTE_G | PTE_N
    );
}
