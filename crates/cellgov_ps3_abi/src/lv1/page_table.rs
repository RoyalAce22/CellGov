//! LV1 hashed page table calls:
//! - the checks and outputs of `read_htab_entries` (hypercall 95);
//! - the checks of `write_htab_entry` (hypercall 1), and the entry that
//!   it stores;
//! - the mapping that `map_htab` (hypercall 122) gives the partition;
//! - the SDR1 and HID6\[LB\] values that a selection of a virtual
//!   address space loads.
//!
//! The facts come from the 4.93 `lv1.self`, decrypted offline and read
//! with `cellgov dev disasm`. No byte of lv1 is committed; these facts
//! are.
//!
//! Both handlers find the address space from r3 in the same way:
//! - A vas id of 0 names the address space of the PPE object that the
//!   calling thread's lv1 context names [OFW-4.93 lv1 a:0x2D1C90]. A thread on the
//!   partition's default address space gets [`HTAB_NO_ENTRY_R3`].
//! - Another id that names no address space of the partition gets
//!   [`HTAB_NO_ENTRY_R3`].
//!
//! lv1 keeps the page table at the real address that the address space
//! holds at +72 of its page table object, and it reads and writes the
//! entries there with translation off [OFW-4.93 lv1 a:0x2D17E8] [OFW-4.93 lv1 a:0x2D2F40].

use crate::hw::ppu_interrupt::doubleword_bit;
use crate::hw::ppu_mmu::{PTES_PER_PTEG, PTE_BYTES, PTE_H, PTE_L, PTE_PP, PTE_V, SMALL_PAGE_LOG2};
use crate::lv1::address_space::HTAB_SIZE_EXP_MIN;
use crate::lv1::status::{zero_extended, LV1_DUPLICATE_ENTRY, LV1_RESOURCE_SHORTAGE};

/// The functions that answer hypercalls 1, 95 and 122 in one firmware's
/// lv1.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PageTableHandlers {
    /// The firmware version, such as `"4.93"`.
    pub firmware: &'static str,
    /// The address of the `write_htab_entry` handler.
    pub write: u32,
    /// The address of the `read_htab_entries` handler.
    pub read: u32,
    /// The address of the `map_htab` worker, which the memory call thunk
    /// calls; see [`MAP_HTAB_RESOURCE_SHORTAGE_R3`].
    pub map: u32,
}

/// Each firmware whose lv1 image shows the three functions, ascending.
///
/// The `write` and `read` addresses are the entries of hypercalls 1 and
/// 95 in the live 4.93 hypercall table
/// ([`super::table::LIVE_TABLE_HANDLERS`]).
pub const PAGE_TABLE_HANDLERS: &[PageTableHandlers] = &[PageTableHandlers {
    firmware: "4.93",
    write: 0x2ED1C0,
    read: 0x2ECAF8,
    map: 0x2DDB20,
}];

/// The handlers of the firmware version `firmware`, or `None` when no
/// lv1 image read shows them.
pub fn page_table_handlers(firmware: &str) -> Option<&'static PageTableHandlers> {
    PAGE_TABLE_HANDLERS
        .iter()
        .find(|handlers| handlers.firmware == firmware)
}

/// The r3 of hypercalls 1 and 95 for a vas id that names no address
/// space of the partition: `LV1_NO_ENTRY` (-6), with r3 marked in the
/// high word [OFW-4.93 lv1 a:0x2ED38C] [OFW-4.93 lv1 a:0x2ECCB4].
///
/// As with `allocate_memory`, bit 32 marks r3, bit 33 r4, bit 34 r5 and
/// bit 35 r6.
pub const HTAB_NO_ENTRY_R3: u64 = 0x0000_0001_FFFF_FFFA;

/// The number of entries in a page table of 2^`exp` bytes.
///
/// Both handlers load the exponent from +116 of the address space, and
/// divide 2^`exp` by 16, the size of an entry [OFW-4.93 lv1 a:0x2ECD98] [OFW-4.93 lv1 a:0x2ED478].
pub const fn page_table_entries(exp: u64) -> u64 {
    (1 << exp) / PTE_BYTES as u64
}

/// The entries that one `read_htab_entries` call returns, from the
/// index in r4 [OFW-4.93 lv1 a:0x2D17EC].
pub const READ_HTAB_ENTRY_COUNT: u64 = 4;

/// The r3 of `read_htab_entries` when the index in r4 plus
/// [`READ_HTAB_ENTRY_COUNT`] is past the entry count:
/// `LV1_ILLEGAL_PARAMETER_VALUE` (-17), with r4 marked [OFW-4.93 lv1 a:0x2ECDA0]
/// [OFW-4.93 lv1 a:0x2ECDD4].
///
/// The handler makes no other check on the index: an index that is not a
/// multiple of 4 reads the 4 entries from it.
pub const READ_HTAB_INDEX_R3: u64 = 0x0000_0002_FFFF_FFEF;

/// The r8 of `read_htab_entries`: the low halfword of doubleword 1 of
/// each entry it reads, the first entry in bits 0:15.
///
/// The reader at [OFW-4.93 lv1 a:0x2D1720] puts doubleword 0 of each entry in r4 to r7.
/// Then it shifts r8 left by 16 and ORs in the halfword at +14 of the
/// entry, once for each entry [OFW-4.93 lv1 a:0x2D1800-0x2D1820]. The halfword holds
/// R, C, WIMG, N and PP. r9 and r10 return 0.
pub const fn read_htab_low_halfwords(doubleword1: [u64; READ_HTAB_ENTRY_COUNT as usize]) -> u64 {
    let mut packed = 0;
    let mut index = 0;
    while index < doubleword1.len() {
        packed = (packed << 16) | (doubleword1[index] & 0xFFFF);
        index += 1;
    }
    packed
}

/// The r3 of `write_htab_entry` when the index in r4 is at or past the
/// entry count: `LV1_ILLEGAL_PARAMETER_VALUE` (-17), with r4 marked
/// [OFW-4.93 lv1 a:0x2ED480] [OFW-4.93 lv1 a:0x2ED654].
///
/// The handler makes its checks in this order, and the first failure is
/// the answer:
/// 1. the address space, [`HTAB_NO_ENTRY_R3`];
/// 2. the index;
/// 3. doubleword 1 in r6, [`WRITE_HTAB_DOUBLEWORD1_R3`];
/// 4. doubleword 0 in r5, [`WRITE_HTAB_DOUBLEWORD0_R3`];
/// 5. the large page size, [`WRITE_HTAB_PAGE_SIZE_R3`] and
///    [`WRITE_HTAB_ALIGNMENT_R3`];
/// 6. a duplicate entry, [`WRITE_HTAB_DUPLICATE_R3`].
pub const WRITE_HTAB_INDEX_R3: u64 = 0x0000_0002_FFFF_FFEF;

/// The r3 of `write_htab_entry` for a doubleword 1 that it refuses: -17,
/// with r6 marked [OFW-4.93 lv1 a:0x2ED640].
///
/// The handler refuses doubleword 1 in these cases:
/// - a 1 in [`WRITE_HTAB_DOUBLEWORD1_RESERVED`] [OFW-4.93 lv1 a:0x2ED488];
/// - a large page entry, L = 1 in doubleword 0, with each bit of
///   [`LARGE_PAGE_FIELD`] set [OFW-4.93 lv1 a:0x2ED498] [OFW-4.93 lv1 a:0x2ED634].
pub const WRITE_HTAB_DOUBLEWORD1_R3: u64 = 0x0000_0008_FFFF_FFEF;

/// The bits of doubleword 1 that `write_htab_entry` refuses: bits 0:1
/// and 52:53.
///
/// The handler rotates doubleword 1 twice under masks, which keeps these
/// four bits, and refuses a nonzero result [OFW-4.93 lv1 a:0x2ED488-0x2ED494].
///
/// [PPC-Book3 p:36 s:4.5.1] Figure 23 leaves doubleword-1 bits 0:1 and 52:53 reserved, around the RPN in bits 2:51.
pub const WRITE_HTAB_DOUBLEWORD1_RESERVED: u64 = 0xC000_0000_0000_0C00;

/// The bits of doubleword 1 that select a large page size: bits 45:51.
///
/// The number of 1 bits from bit 51 up, before the first 0 bit, is the
/// index of the page size in the address space [OFW-4.93 lv1 a:0x2D38D8-0x2D38F4].
pub const LARGE_PAGE_FIELD: u64 = 0x0000_0000_0007_F000;

/// The r3 of `write_htab_entry` for a doubleword 0 with a 1 in
/// [`WRITE_HTAB_DOUBLEWORD0_RESERVED`]: -17, with r5 marked [OFW-4.93 lv1 a:0x2ED620].
///
/// The handler counts the leading zeros of doubleword 0 with its low 7
/// bits cleared, and compares 79 less the count with 65, the value of the
/// function at [OFW-4.93 lv1 a:0x2B9CE8] [OFW-4.93 lv1 a:0x2ED4A8-0x2ED4BC].
pub const WRITE_HTAB_DOUBLEWORD0_R3: u64 = 0x0000_0004_FFFF_FFEF;

/// The bits of doubleword 0 that `write_htab_entry` refuses: bits 0:14.
pub const WRITE_HTAB_DOUBLEWORD0_RESERVED: u64 = 0xFFFE_0000_0000_0000;

/// The r3 of `write_htab_entry` for a large page whose page size index
/// is not below the address space's page size count:
/// `LV1_NO_ENTRY` (-6), with r6 marked.
///
/// The worker at [OFW-4.93 lv1 a:0x2D368C] returns the code 0x00DC817DB6321201
/// [OFW-4.93 lv1 a:0x2D395C], and the handler gives this r3 for it [OFW-4.93 lv1 a:0x2ED5B0].
pub const WRITE_HTAB_PAGE_SIZE_R3: u64 = 0x0000_0008_FFFF_FFFA;

/// The r3 of `write_htab_entry` for a large page whose address, less its
/// large page bits, is not a multiple of the page size:
/// `LV1_ALIGNMENT_ERROR` (-23), with r6 marked.
///
/// The worker returns the code 0x00DC817DB6321202 [OFW-4.93 lv1 a:0x2D393C], and the
/// handler gives this r3 for it [OFW-4.93 lv1 a:0x2ED59C].
pub const WRITE_HTAB_ALIGNMENT_R3: u64 = 0x0000_0008_FFFF_FFE9;

/// The r3 of `write_htab_entry` for a valid entry that duplicates
/// another valid entry: `LV1_DUPLICATE_ENTRY` (-7), zero-extended.
///
/// The writer at [OFW-4.93 lv1 a:0x2D2EC8] returns 4, and the handler gives this r3 for
/// it [OFW-4.93 lv1 a:0x2ED53C]. See [`duplicates`] for the two searches.
pub const WRITE_HTAB_DUPLICATE_R3: u64 = zero_extended(LV1_DUPLICATE_ENTRY);

/// The page size index of a large page entry with doubleword 1
/// `doubleword1`: the number of 1 bits from bit 51 up.
///
/// The caller has refused a [`LARGE_PAGE_FIELD`] with each bit set.
pub const fn large_page_index(doubleword1: u64) -> u32 {
    (doubleword1 >> SMALL_PAGE_LOG2).trailing_ones()
}

/// The large page bits of a page size index `index`: the `index` bits
/// from bit 51 up.
///
/// The worker subtracts them from the page's address before the
/// alignment check, and the writer ORs them into the stored entry
/// [OFW-4.93 lv1 a:0x2D3918] [OFW-4.93 lv1 a:0x2D3120].
pub const fn large_page_bits(index: u32) -> u64 {
    (1 << (SMALL_PAGE_LOG2 + index)) - (1 << SMALL_PAGE_LOG2)
}

/// The bits of doubleword 0 that the duplicate searches ignore: bits
/// 57:60, the software bits.
///
/// The writer clears them from the XOR of the two doublewords [OFW-4.93 lv1 a:0x2D2F58]
/// [OFW-4.93 lv1 a:0x2D2FB8].
pub const DUPLICATE_IGNORED: u64 = 0x78;

/// Whether `existing`, doubleword 0 of an entry that the page table
/// holds, duplicates `new`, a valid doubleword 0 that a write puts in
/// another slot.
///
/// The writer searches two page table entry groups
/// [OFW-4.93 lv1 a:0x2D2F24-0x2D2FC4]:
/// - the group of the written slot, past the slot itself, for an entry
///   equal to `new`;
/// - the other group of the hash, for an entry that differs from `new`
///   only in H.
///
/// `same_group` names the group of the written slot.
pub const fn duplicates(existing: u64, new: u64, same_group: bool) -> bool {
    let differ = (existing ^ new) & !DUPLICATE_IGNORED;
    if same_group {
        differ == 0
    } else {
        differ == PTE_H
    }
}

/// The index of the first entry of the other group of the hash, for the
/// entry index `index` in a page table of 2^`exp` bytes.
///
/// The writer XORs the group index with the group count less 1
/// [OFW-4.93 lv1 a:0x2D2F78-0x2D2F90].
pub const fn other_group(index: u64, exp: u64) -> u64 {
    let groups = page_table_entries(exp) / PTES_PER_PTEG as u64;
    (index & !(PTES_PER_PTEG as u64 - 1)) ^ ((groups - 1) * PTES_PER_PTEG as u64)
}

/// The bits of doubleword 1 that the writer keeps in the stored entry:
/// bits 0:1, 52:56 and 61:63.
///
/// It clears WIMG and the real page number, and replaces them
/// [OFW-4.93 lv1 a:0x2D3104-0x2D312C].
pub const STORED_DOUBLEWORD1_KEPT: u64 = 0xC000_0000_0000_0F87;

/// The M bit of doubleword 1, bit 59: memory coherence.
pub const PTE_M: u64 = doubleword_bit(59);

/// The highest real address for which the writer stores WIMG = 0b0010,
/// [`PTE_M`] alone [OFW-4.93 lv1 a:0x2D3108] [OFW-4.93 lv1 a:0x2D3110].
///
/// For a higher address, the writer stores the WIMG of an I/O range
/// [OFW-4.93 lv1 a:0x2D3178-0x2D32A0].
pub const STORED_MEMORY_LIMIT: u64 = 0xFFFF_FFFF;

/// Doubleword 1 that the writer stores for the doubleword 1
/// `doubleword1` of a write, a page at the real address `real`, and the
/// large page bits `large_page_bits`.
///
/// `real` is at or below [`STORED_MEMORY_LIMIT`]. For an invalid entry
/// (V = 0), the writer maps no page and `real` is 0 [OFW-4.93 lv1 a:0x2D3050].
pub const fn stored_doubleword1(doubleword1: u64, real: u64, large_page_bits: u64) -> u64 {
    real | large_page_bits | (doubleword1 & STORED_DOUBLEWORD1_KEPT) | PTE_M
}

/// Whether a doubleword 0 is a valid entry. The worker maps the page
/// only for a valid entry [OFW-4.93 lv1 a:0x2D36F0].
pub const fn valid(doubleword0: u64) -> bool {
    doubleword0 & PTE_V != 0
}

/// Whether a doubleword 0 is a large page entry, L = 1 [OFW-4.93 lv1 a:0x2D369C].
pub const fn large(doubleword0: u64) -> bool {
    doubleword0 & PTE_L != 0
}

/// Whether a doubleword 1 maps its page read only: PP = 0b11.
///
/// The writer gives this flag to the translation of the page's region
/// [OFW-4.93 lv1 a:0x2D308C-0x2D30B0].
pub const fn read_only(doubleword1: u64) -> bool {
    doubleword1 & PTE_PP == PTE_PP
}

/// The r3 of `map_htab` when lv1 cannot allocate the mapping:
/// `LV1_RESOURCE_SHORTAGE` (-2), zero-extended [OFW-4.93 lv1 a:0x2DDDF0].
///
/// Hypercall 122 reaches lv1's memory call thunk at
/// [OFW-4.93 lv1 a:0x2C89CC], which calls the function that the memory
/// call table holds for the number. The function at
/// [OFW-4.93 lv1 a:0x2C8B90] fills that table at run time. The stub at
/// [OFW-4.93 lv1 a:0x2DDF8C] passes it the 2-entry array at
/// [OFW-4.93 lv1 a:0x362258], which gives 122 the descriptor
/// [OFW-4.93 lv1 a:0x347A48] and so the worker at [OFW-4.93 lv1 a:0x2DDB20].
///
/// The worker takes the vas id in r3, and finds the address space as the
/// other page table calls do: a failure gets [`HTAB_NO_ENTRY_R3`]
/// [OFW-4.93 lv1 a:0x2DDC90]. Then it makes a memory region over the page table
/// [OFW-4.93 lv1 a:0x2DDD94-0x2DDE38]:
/// - The region's logical partition address encodes the page table's
///   size exponent, and takes the value of the partition's region counter
///   [OFW-4.93 lv1 a:0x2CEC78]. `allocate_memory` takes its addresses from the same
///   counter [OFW-4.93 lv1 a:0x2DEA94]; see [`super::memory::region_address`].
/// - r4 gets the address, and r3 gets 0.
///
/// The thunk copies r4 to r10 from its own stack frame
/// [OFW-4.93 lv1 a:0x2C8B14-0x2C8B34]. The worker writes only r4, so r5 to r10 hold data that lv1
/// does not set.
pub const MAP_HTAB_RESOURCE_SHORTAGE_R3: u64 = zero_extended(LV1_RESOURCE_SHORTAGE);

/// SDR1 for a page table at the real address `table` of 2^`exp` bytes:
/// HTABSIZE is `exp` less 18.
///
/// A selection of an address space stores its page table object at +56
/// of the PPE object [OFW-4.93 lv1 a:0x2C51BC], then calls [OFW-4.93 lv1 a:0x2C37E0]. That function
/// loads SDR1 with the address at +72 ORed with the exponent at +100
/// less 18 [OFW-4.93 lv1 a:0x2C3A40-0x2C3A54].
pub const fn select_sdr1(table: u64, exp: u64) -> u64 {
    table | (exp - HTAB_SIZE_EXP_MIN)
}

/// HID6\[LB\], as a 4-bit value with LB bit 0 most significant, for an
/// address space with `count` page sizes and the page shifts `shifts`.
///
/// The function at [OFW-4.93 lv1 a:0x2C37E0] sets LB from the first two page shifts
/// [OFW-4.93 lv1 a:0x2C392C-0x2C3984]:
/// - The first shift sets LB bits 0:1, which size the pages with LP = 0.
/// - The second sets LB bits 2:3, for LP = 1.
/// - Each field is 2 less the shift less 16, divided by 4: '10' for a
///   shift of 16, '01' for 20 and '00' for 24.
/// - A field with no shift is '00'.
pub const fn select_hid6_lb(count: u64, shifts: u64) -> u8 {
    let first = if count > 0 { lb_field(shifts, 0) } else { 0 };
    let second = if count > 1 { lb_field(shifts, 1) } else { 0 };
    (first << 2) | second
}

/// The LB field for page shift `index` of `shifts`.
const fn lb_field(shifts: u64, index: u32) -> u8 {
    let shift = ((shifts >> (56 - 8 * index)) & 0xFF) as i64;
    (2 - ((shift - 16) >> 2)) as u8 & 0b11
}

#[cfg(test)]
#[path = "tests/page_table_tests.rs"]
mod tests;
