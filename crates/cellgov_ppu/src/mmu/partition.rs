//! The translation resources the hypervisor sets for a partition.

use cellgov_ps3_abi::hw::ppu_mmu::{
    large_page_log2, HTABSIZE_MAX, PPE_HTABORG_MAX, PPE_HTABSIZE_MAX, RMLR_LIMIT_LOG2,
    SDR1_HTABORG, SDR1_HTABSIZE,
};

/// SDR1, the real mode limit and offset, and the large page sizes of one partition.
///
/// These are hypervisor resources: the partition's own code reads and
/// writes none of them. CellGov is the hypervisor, so CellGov sets them.
/// The partition runs with LPES1 = 1, so offset real mode controls its
/// accesses with translation off.
///
/// [PPC-Book3 p:37 s:4.5.2] SDR1 is a hypervisor resource.
/// [PPC-Book3 p:29 s:4.2.6] With MSR\[HV\] = 0 and LPES1 = 1, the RMLR and RMOR control real addressing mode.
/// [CBE-Handbook p:89 s:4.2.6.6] Each partition can have its own HID6\[LB\], which the hypervisor changes only when it creates or switches to a partition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Partition {
    sdr1: u64,
    rm_limit: u64,
    rm_offset: u64,
    lb: u8,
}

/// Why [`Partition::new`] refused a set of resources.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum PartitionRefusal {
    /// SDR1 names no page table the PPE allows.
    ///
    /// SDR1 has one of:
    /// - a 1 bit outside HTABORG and HTABSIZE;
    /// - an HTABSIZE or HTABORG past the PPE's limits;
    /// - a 1 bit in the low-order HTABORG bits that HTABSIZE covers.
    #[error("SDR1 0x{0:016x} names no page table")]
    Sdr1(u64),
    /// The real mode limit is not 2^j with 12 <= j <= 62.
    #[error("real mode limit 0x{0:016x} is not a supported RMLR value")]
    RealModeLimit(u64),
    /// The real mode offset names no block the partition allows.
    ///
    /// The offset is one of:
    /// - not a multiple of the limit;
    /// - the start of a block that reaches past the 62-bit real address
    ///   space.
    #[error("real mode offset 0x{0:016x} is not a multiple of the limit")]
    RealModeOffset(u64),
    /// HID6\[LB\] names no pair of large page sizes.
    ///
    /// LB has one of:
    /// - a 1 bit above its four bits;
    /// - a field that selects the reserved size.
    #[error("HID6 LB 0x{0:x} selects a reserved large page size")]
    LargePages(u8),
}

impl Partition {
    /// The resources a partition starts with, until the hypervisor sets its own.
    ///
    /// - SDR1 = 0.
    /// - Real mode covers every EA (bits 2:63), at offset 0.
    /// - Large pages are 16 MB (LP = 0) and 64 KB (LP = 1).
    ///
    /// [PPC-Book3 p:37 s:4.5.2] SDR1 = 0 names the smallest page table, 256 KB at real address 0.
    pub const INITIAL: Self = Self {
        sdr1: 0,
        rm_limit: 1 << RMLR_LIMIT_LOG2.1,
        rm_offset: 0,
        lb: 0b0010,
    };

    /// The partition with these resources.
    ///
    /// The real mode offset must be a multiple of the limit. The
    /// architecture also allows offsets of a smaller granule; CellGov's
    /// hypervisor uses none of them, so real addressing mode maps one
    /// contiguous block.
    ///
    /// [PPC-Book3 p:37 s:4.5.2] HTABSIZE is at most 28, and HTABORG has at least HTABSIZE low-order 0 bits.
    /// [CBE-Handbook p:87 s:4.2.6.4] The PPE allows at most HTABSIZE 24 and HTABORG x'FFFFFF'.
    /// [PPC-Book3 p:30 s:4.2.6.2] The supported limit values are 2^j, where 12 <= j <= 62.
    ///
    /// # Errors
    ///
    /// The [`PartitionRefusal`] of the first resource that fails its
    /// check.
    pub fn new(sdr1: u64, rm_limit: u64, rm_offset: u64, lb: u8) -> Result<Self, PartitionRefusal> {
        let htabsize = sdr1 & SDR1_HTABSIZE;
        let origin_shift = SDR1_HTABORG.trailing_zeros();
        let index_bits = ((1u64 << htabsize.min(HTABSIZE_MAX)) - 1) << origin_shift;
        if sdr1 & !(SDR1_HTABORG | SDR1_HTABSIZE) != 0
            || htabsize > PPE_HTABSIZE_MAX
            || (sdr1 & SDR1_HTABORG) >> origin_shift > PPE_HTABORG_MAX
            || sdr1 & index_bits != 0
        {
            return Err(PartitionRefusal::Sdr1(sdr1));
        }
        let (lo, hi) = RMLR_LIMIT_LOG2;
        if !rm_limit.is_power_of_two() || !(lo..=hi).contains(&rm_limit.trailing_zeros()) {
            return Err(PartitionRefusal::RealModeLimit(rm_limit));
        }
        if !rm_offset.is_multiple_of(rm_limit)
            || rm_offset
                .checked_add(rm_limit)
                .is_none_or(|end| end > 1 << hi)
        {
            return Err(PartitionRefusal::RealModeOffset(rm_offset));
        }
        if lb > 0xF || large_page_log2(lb >> 2).is_none() || large_page_log2(lb).is_none() {
            return Err(PartitionRefusal::LargePages(lb));
        }
        Ok(Self {
            sdr1,
            rm_limit,
            rm_offset,
            lb,
        })
    }

    /// SDR1: the page table's real address and size.
    pub fn sdr1(&self) -> u64 {
        self.sdr1
    }

    /// The real mode limit: the first EA (bits 2:63) real mode refuses.
    pub fn rm_limit(&self) -> u64 {
        self.rm_limit
    }

    /// The real mode offset, ORed into EA bits 2:63.
    pub fn rm_offset(&self) -> u64 {
        self.rm_offset
    }

    /// HID6\[LB\] as a 4-bit value, LB bit 0 most significant.
    pub fn lb(&self) -> u8 {
        self.lb
    }

    /// Log2 of the size of a large page with selector `lp`.
    ///
    /// [CBE-Handbook p:89 s:4.2.6.6] LB bits 0:1 size the pages with LP = 0 and bits 2:3 those with LP = 1.
    pub fn large_page_log2(&self, lp: bool) -> u32 {
        let field = if lp { self.lb } else { self.lb >> 2 };
        // `new` refuses a reserved field, so the decode always holds.
        large_page_log2(field).unwrap_or(24)
    }
}
