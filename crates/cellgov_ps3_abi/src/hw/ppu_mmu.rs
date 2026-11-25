//! The PPE's address translation: real addressing mode, the segment
//! lookaside buffer's flags, the hashed page table and its entries, and
//! the large page sizes.
//!
//! The bit numbers are the ones the ISA prints: bit 0 is the most
//! significant bit of the 64-bit register or doubleword.

use super::ppu_interrupt::doubleword_bit;

/// The effective-address bits real addressing mode ignores outside hypervisor state: bits 0:1.
///
/// [PPC-Book3 p:29 s:4.2.6] Real addressing mode ignores EA bit 1, and with MSR\[HV\] = 0 and LPES1 = 1 the offset real mode mechanism controls the access.
/// [PPC-Book3 p:30 s:4.2.6.2] The offset real mode mechanism compares EA bits 2:63 with the RMLR limit and ORs them with the RMOR offset.
pub const REAL_MODE_IGNORED_EA: u64 = 0xC000_0000_0000_0000;

/// The smallest and largest log2 of an RMLR limit.
///
/// [PPC-Book3 p:30 s:4.2.6.2] The supported limit values are of the form 2^j, where 12 <= j <= 62.
pub const RMLR_LIMIT_LOG2: (u32, u32) = (12, 62);

/// The largest real mode limit the PPE supports: 256 GB, which LPCR\[RMLS\] = 0000 selects.
///
/// [CBE-Handbook p:336 s:11.2] Table 11-1: the RMLS values select limits from 1 MB to 256 GB, and RMLS 0000 selects 256 GB.
pub const PPE_RM_LIMIT_MAX: u64 = 1 << 38;

/// Log2 of the segment size: 256 MB.
///
/// [PPC-Book3 p:26 s:4.2] Segment size is 2^28 bytes.
pub const SEGMENT_LOG2: u32 = 28;

/// The SLB's Ks bit: the key in privileged state, bit 52 of the virtual half.
///
/// [PPC-Book3 p:51 s:4.11.3.1] `slbmte` loads Ks, Kp, N, L and C from RS bits 52, 53, 54, 55 and 56.
pub const SLB_KS: u64 = doubleword_bit(52);

/// The SLB's Kp bit: the key in problem state. See [`SLB_KS`].
pub const SLB_KP: u64 = doubleword_bit(53);

/// The SLB's N bit: no-execute segment. See [`SLB_KS`].
pub const SLB_N: u64 = doubleword_bit(54);

/// The SLB's L bit: the segment's pages are large. See [`SLB_KS`].
pub const SLB_L: u64 = doubleword_bit(55);

/// Log2 of the small page size: 4 KB.
///
/// [PPC-Book3 p:33 s:4.4] The L bit selects between 4 KB pages (p = 12) and large pages.
pub const SMALL_PAGE_LOG2: u32 = 12;

/// The HTABORG field of SDR1, bits 2:45: the page table's real address.
///
/// [PPC-Book3 p:37 s:4.5.2] HTABORG holds the high-order 44 bits of the 62-bit real address of the page table, and HTABSIZE in bits 59:63 the number of hash bits beyond 11 that index it, at most 28.
pub const SDR1_HTABORG: u64 = 0x3FFF_FFFF_FFFC_0000;

/// The HTABSIZE field of SDR1, bits 59:63. See [`SDR1_HTABORG`].
pub const SDR1_HTABSIZE: u64 = 0x1F;

/// The largest HTABSIZE. See [`SDR1_HTABORG`].
pub const HTABSIZE_MAX: u64 = 28;

/// The largest HTABSIZE the PPE allows.
///
/// [CBE-Handbook p:87 s:4.2.6.4] The PPE supports only a 42-bit RA, so SDR1 allows at most HTABSIZE 24 and HTABORG x'FFFFFF'.
pub const PPE_HTABSIZE_MAX: u64 = 24;

/// The largest HTABORG field value the PPE allows. See [`PPE_HTABSIZE_MAX`].
pub const PPE_HTABORG_MAX: u64 = 0xFF_FFFF;

/// Bits in a PPE real address. See [`PPE_HTABSIZE_MAX`].
pub const PPE_REAL_ADDRESS_BITS: u32 = 42;

/// The hash bits that index the smallest page table.
///
/// [PPC-Book3 p:37 s:4.5.2] At least 11 bits from the hash index the page table: 2^11 PTEGs of 128 bytes.
pub const HASH_INDEX_MIN_BITS: u32 = 11;

/// Bits in the page table hash.
///
/// [PPC-Book3 p:38 s:4.5.3] The primary hash is 39 bits: VPN bits 13:51 XORed with the low-order 28-p bits of the VPN, and the secondary hash is its one's complement.
pub const HASH_BITS: u32 = 39;

/// Bytes in a page table entry group.
///
/// [PPC-Book3 p:36 s:4.5.1] A PTEG holds 8 PTEs of 16 bytes each, so it is 128 bytes long.
pub const PTEG_BYTES: usize = 128;

/// Page table entries in a group. See [`PTEG_BYTES`].
pub const PTES_PER_PTEG: usize = 8;

/// Bytes in a page table entry. See [`PTEG_BYTES`].
pub const PTE_BYTES: usize = 16;

/// The PTE's V bit: bit 63 of doubleword 0.
///
/// [PPC-Book3 p:36 s:4.5.1] Doubleword 0 holds AVPN in bits 0:56, L in bit 61, H in bit 62 and V in bit 63; doubleword 1 holds RPN in bits 2:51, R in 55, C in 56, WIMG in 57:60, N in 61 and PP in 62:63.
pub const PTE_V: u64 = doubleword_bit(63);

/// The PTE's H bit: the entry is in its secondary group. See [`PTE_V`].
pub const PTE_H: u64 = doubleword_bit(62);

/// The PTE's L bit: the page is large. See [`PTE_V`].
pub const PTE_L: u64 = doubleword_bit(61);

/// The doubleword-0 bits a PTE match compares on the PPE: AVPN bits 15:56.
/// A match on a page of 16 MB or more compares fewer of the low-order bits.
///
/// [CBE-Handbook p:88 s:4.2.6.5] The PPE does not implement PTE bits 0:15 and matches as if they were 0.
/// [CBE-Handbook p:86 s:4.2.6.2] Table 4-2: PTE bits 0:14 are reserved and bits 15:56 are the AVPN.
/// [CBE-Handbook p:84 s:4.2.5.2] The SLB does not implement VSID bits 0:14, so it keeps VSID bit 15.
/// [PPC-Book3 p:38 s:4.5.3] A match compares AVPN bits 0:51 with VA bits 0:51 and, for p < 28, AVPN bits 52:51+q with VA bits 52:51+q, where q = min(5, 28-p).
///
/// CBE-Handbook pages 88 and 86 disagree on bit 15. Both refuse a match
/// of a VA with bit 15 = 1 against a PTE with bit 15 = 0. A match
/// therefore cannot ignore the bit, so the mask follows Table 4-2.
pub const PPE_PTE_AVPN: u64 = 0x0001_FFFF_FFFF_FF80;

/// The PTE's RPN field: bits 2:51 of doubleword 1. See [`PTE_V`].
pub const PTE_RPN: u64 = 0x3FFF_FFFF_FFFF_F000;

/// The RPN bits the PPE implements: bits 22:51 of doubleword 1, the
/// page bits of a 42-bit real address.
///
/// [CBE-Handbook p:86 s:4.2.6.2] Table 4-2: PTE bits 64:85 are reserved and bits 86:115 are the RPN.
/// [CBE-Handbook p:79 s:4.2.1] Table 4-1: the PPE does not implement real address bits 2:21 and treats them as 0.
pub const PPE_PTE_RPN: u64 = 0x0000_03FF_FFFF_F000;

/// The PTE's reference bit: R, bit 55 of doubleword 1. See [`PTE_V`].
pub const PTE_R: u64 = doubleword_bit(55);

/// The PTE's change bit: C, bit 56 of doubleword 1. See [`PTE_V`].
pub const PTE_C: u64 = doubleword_bit(56);

/// The byte of a PTE that holds R: byte 14, whose low-order bit is
/// doubleword 1 bit 55.
///
/// [PPC-Book3 p:44 s:4.9] The processor can set R or C with a store to a byte, halfword, word or doubleword. See [`PTE_V`] for the bit numbers.
pub const PTE_R_BYTE: usize = 14;

/// R's mask in [`PTE_R_BYTE`].
pub const PTE_R_BYTE_MASK: u8 = 0x01;

/// The byte of a PTE that holds C: byte 15, whose high-order bit is
/// doubleword 1 bit 56. See [`PTE_R_BYTE`].
pub const PTE_C_BYTE: usize = 15;

/// C's mask in [`PTE_C_BYTE`].
pub const PTE_C_BYTE_MASK: u8 = 0x80;

/// The PTE's caching-inhibited bit: I, bit 58 of doubleword 1.
///
/// [CBE-Handbook p:89 s:4.2.6.7] The WIMG bits are write-through, caching-inhibited, memory coherency and guarded, in that order. See [`PTE_V`] for their place.
pub const PTE_I: u64 = doubleword_bit(58);

/// The PTE's guarded bit: G, bit 60 of doubleword 1. See [`PTE_I`].
pub const PTE_G: u64 = doubleword_bit(60);

/// The PTE's N bit: no-execute page. See [`PTE_V`].
pub const PTE_N: u64 = doubleword_bit(61);

/// The PTE's PP field: bits 62:63 of doubleword 1. See [`PTE_V`].
pub const PTE_PP: u64 = 0x3;

/// The PPE's large page selector in a PTE with L = 1: bit 51 of
/// doubleword 1, the low-order RPN bit.
///
/// [CBE-Handbook p:88 s:4.2.6.5] The least significant RPN bit doubles as PTE\[LP\] when PTE\[L\] = 1, and a match needs PTE\[LP\] = SLBE\[LP\].
pub const PPE_PTE_LP: u64 = doubleword_bit(51);

/// Log2 of the large page size that two bits of HID6\[LB\] select, or
/// `None` for the reserved encoding.
///
/// [CBE-Handbook p:89 s:4.2.6.6] Table 4-3: LB bits 0:1 size the pages with LP = 0 and bits 2:3 those with LP = 1; '10' is 64 KB, '01' 1 MB, '00' 16 MB and '11' reserved.
pub const fn large_page_log2(lb_field: u8) -> Option<u32> {
    match lb_field & 0b11 {
        0b10 => Some(16),
        0b01 => Some(20),
        0b00 => Some(24),
        _ => None,
    }
}

#[cfg(test)]
#[path = "tests/ppu_mmu_tests.rs"]
mod tests;
