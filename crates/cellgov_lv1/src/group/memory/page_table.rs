//! Hypercalls 1, 95 and 122, `write_htab_entry`, `read_htab_entries` and
//! `map_htab`, and the storage that holds the page tables of the address
//! spaces that the hypervisor constructs.
//!
//! lv1 keeps the page table of each address space in its own memory, at
//! a real address that the address space records and that no call gives
//! the partition. CellGov keeps each table in guest storage from
//! [`PAGE_TABLE_STORAGE_BASE`], at the real address that [`placement`]
//! gives it, so that the PPU's table search reads it through SDR1. A
//! selection of an address space loads SDR1 with the address of its
//! table.
//!
//! The answers read the entries from the storage and change them with
//! memory writes. The PPU writes R and C into the same bytes, so a read
//! returns them.

use cellgov_effects::{Effect, WritePayload};
use cellgov_mem::{be::read_u64, ByteRange, GuestAddr, GuestMemory, PageSize, Region};
use cellgov_ps3_abi::hw::ppu_mmu::{
    PTEG_BYTES, PTES_PER_PTEG, PTE_BYTES, PTE_RPN, SMALL_PAGE_LOG2,
};
use cellgov_ps3_abi::lv1::address_space::{htab_size, HTAB_SIZE_EXP_MAX};
use cellgov_ps3_abi::lv1::call::{MAP_HTAB, READ_HTAB_ENTRIES, WRITE_HTAB_ENTRY};
use cellgov_ps3_abi::lv1::memory::game_os_region_counter_at_boot;
use cellgov_ps3_abi::lv1::page_table::{
    duplicates, large, large_page_bits, large_page_index, other_group, page_table_entries,
    page_table_handlers, read_htab_low_halfwords, select_hid6_lb, select_sdr1, stored_doubleword1,
    stored_io_doubleword1, valid, HTAB_NO_ENTRY_R3, LARGE_PAGE_FIELD, READ_HTAB_ENTRY_COUNT,
    READ_HTAB_INDEX_R3, STORED_MEMORY_LIMIT, WRITE_HTAB_ALIGNMENT_R3, WRITE_HTAB_DOUBLEWORD0_R3,
    WRITE_HTAB_DOUBLEWORD0_RESERVED, WRITE_HTAB_DOUBLEWORD1_R3, WRITE_HTAB_DOUBLEWORD1_RESERVED,
    WRITE_HTAB_DUPLICATE_R3, WRITE_HTAB_INDEX_R3, WRITE_HTAB_PAGE_SIZE_R3,
};
use cellgov_ps3_abi::lv1::status::{zero_extended, LV1_SUCCESS};

use super::address_space::{AddressSpace, AddressSpaceChange, AddressSpaceState};
use super::allocator::{place, HeldRegions};
use super::mmio_page::{self, MmioPage};
use super::region as memory;
use super::region::MemoryState;
use super::storage::{self, AllocatedPage};
use crate::answer::{HypercallAnswer, HypercallDisposition, HypercallRefusal, PageTableLoad};
use crate::group::device_bus::DeviceBusState;
use crate::group::hypercall::Hypercall;
use crate::group::spe::shadow::{region_page, SpeRegionPage};
use crate::group::spe::LogicalSpeState;
use crate::request::HypercallRequest;

/// The hypercalls this module answers.
pub(crate) const HYPERCALLS: &[Hypercall] = &[
    Hypercall {
        number: WRITE_HTAB_ENTRY,
        name: "write_htab_entry",
        cost: None,
        handler: |request, input| {
            let memory = input.groups.memory();
            write_entry(
                request,
                input.firmware,
                memory.spaces(),
                input.groups.spe().spes(),
                memory.regions(),
                input.groups.device_bus(),
                input.storage,
            )
        },
    },
    Hypercall {
        number: READ_HTAB_ENTRIES,
        name: "read_htab_entries",
        cost: None,
        handler: |request, input| {
            read_entries(
                request,
                input.firmware,
                input.groups.memory().spaces(),
                input.storage,
            )
        },
    },
    Hypercall {
        number: MAP_HTAB,
        name: "map_htab",
        cost: None,
        handler: |request, input| {
            let memory = input.groups.memory();
            map_table(
                request,
                input.firmware,
                memory.spaces(),
                memory.regions(),
                &HeldRegions::of(input.groups),
            )
        },
    },
];

/// The real address of the storage that holds the hypervisor's page
/// tables.
///
/// CellGov chooses the address, with these properties:
/// - It lies above the partition's real mode region.
/// - It is at or below [`STORED_MEMORY_LIMIT`], so an entry that maps
///   the table stores the WIMG of memory, M.
/// - It is a multiple of the largest page table, as SDR1 needs.
pub const PAGE_TABLE_STORAGE_BASE: u64 = 0x1000_0000;

/// The size of the storage: the largest page table that
/// `construct_virtual_address_space` makes.
pub const PAGE_TABLE_STORAGE_BYTES: u64 = htab_size(HTAB_SIZE_EXP_MAX);

/// The region that holds the hypervisor's page tables, for a boot that
/// runs the kernel under CellGov's hypervisor.
pub fn page_table_storage() -> Region {
    Region::new(
        PAGE_TABLE_STORAGE_BASE,
        PAGE_TABLE_STORAGE_BYTES as usize,
        "lv1 page table",
        PageSize::Page64K,
    )
}

/// The real address of a new page table of 2^`htab_size_exp` bytes whose
/// storage starts at or after `end`, or `None` when the storage has no
/// room for it.
///
/// The table starts at the first multiple of its size, as SDR1 needs
/// [PPC-Book3 p:36 s:4.5.1].
pub(crate) fn placement(end: u64, htab_size_exp: u64) -> Option<u64> {
    let size = htab_size(htab_size_exp);
    let table = end.checked_next_multiple_of(size)?;
    let storage_end = PAGE_TABLE_STORAGE_BASE + PAGE_TABLE_STORAGE_BYTES;
    (table.checked_add(size)? <= storage_end).then_some(table)
}

/// Whether `storage` holds the page table of `space`, whose real address
/// is `table`.
pub(crate) fn holds_table(storage: &GuestMemory, space: &AddressSpace, table: u64) -> bool {
    storage.holds(table, htab_size(space.htab_size_exp))
}

/// The registers that a selection of `space`, whose page table is at the
/// real address `table`, loads on the calling thread.
///
/// lv1 loads SDR1 from the table address that the address space records
/// [OFW-4.93 lv1 a:0x2C3A40-0x2C3A54].
pub(crate) fn load(space: &AddressSpace, table: u64) -> PageTableLoad {
    PageTableLoad {
        sdr1: select_sdr1(table, space.htab_size_exp),
        lb: select_hid6_lb(space.page_size_count, space.page_shifts),
    }
}

fn refuse(request: &HypercallRequest, what: &'static str) -> HypercallDisposition {
    HypercallDisposition::Refused(HypercallRefusal::UnmodelledInput {
        number: request.number,
        unit: request.unit,
        pc: request.pc,
        what,
    })
}

/// lv1's own r3 with r4 to r10 cleared.
pub(crate) fn lv1_answer(r3: u64) -> HypercallDisposition {
    HypercallDisposition::Answered(HypercallAnswer::lv1_status(r3, []))
}

/// Why a page table call names no address space that the hypervisor
/// holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NoSpace {
    /// lv1 answers [`HTAB_NO_ENTRY_R3`].
    Lv1NoEntry,
    /// CellGov refuses the call for this reason.
    Refused(&'static str),
}

impl NoSpace {
    /// The disposition of `request`: lv1's r3 with r4 to r10 cleared, or
    /// the refusal.
    pub(crate) fn disposition(self, request: &HypercallRequest) -> HypercallDisposition {
        match self {
            Self::Lv1NoEntry => lv1_answer(HTAB_NO_ENTRY_R3),
            Self::Refused(what) => refuse(request, what),
        }
    }
}

/// The address space that the vas id in r3 of `request` names.
///
/// A vas id of 0 names the selected address space.
///
/// # Errors
///
/// - [`NoSpace::Lv1NoEntry`] for a vas id of 0 before a selection: the
///   caller is on the partition's default address space.
/// - [`NoSpace::Refused`] when no lv1 image read shows the handlers for
///   `firmware`.
/// - [`NoSpace::Refused`] when the hypervisor did not construct the
///   address space.
pub(crate) fn named_space<'a>(
    request: &HypercallRequest,
    firmware: Option<&str>,
    spaces: &'a AddressSpaceState,
) -> Result<&'a AddressSpace, NoSpace> {
    if firmware.and_then(page_table_handlers).is_none() {
        return Err(NoSpace::Refused(
            "a page table call at a firmware whose lv1 handler is not read",
        ));
    }
    let vas_id = match (request.r(3), spaces.selected()) {
        // lv1 finds the default address space on the caller's PPE object
        // and answers no entry [OFW-4.93 lv1 a:0x2D1C90]; see HTAB_NO_ENTRY_R3.
        (0, None) => return Err(NoSpace::Lv1NoEntry),
        (0, Some(selected)) => selected,
        (vas_id, _) => vas_id,
    };
    spaces.space(vas_id).ok_or(NoSpace::Refused(
        "a page table call on an address space that this hypervisor did not construct",
    ))
}

/// The real address of entry `index` of the page table at the real
/// address `table`.
fn entry_address(table: u64, index: u64) -> u64 {
    table + index * PTE_BYTES as u64
}

/// Hypercall 95: four entries of the page table from the index in r4.
///
/// r4 to r7 get doubleword 0 of each entry. r8 gets the low halfword of
/// doubleword 1 of each entry, with R and C as the table holds them. r9
/// and r10 get 0.
///
/// An index whose four entries pass the end of the table gets lv1's
/// [`READ_HTAB_INDEX_R3`], with r4 to r10 cleared. lv1 checks the
/// address space first, and CellGov answers it only as [`named_space`]
/// allows.
///
/// The call is a refusal in these cases:
/// - [`named_space`] refuses it;
/// - the index plus 4 overflows, which lv1's check does not catch;
/// - no storage holds the entries.
pub(crate) fn read_entries(
    request: &HypercallRequest,
    firmware: Option<&str>,
    spaces: &AddressSpaceState,
    storage: &GuestMemory,
) -> HypercallDisposition {
    let space = match named_space(request, firmware, spaces) {
        Ok(space) => space,
        Err(missing) => return missing.disposition(request),
    };
    let index = request.r(4);
    let Some(end) = index.checked_add(READ_HTAB_ENTRY_COUNT) else {
        return refuse(
            request,
            "a page table read whose index plus 4 overflows a doubleword",
        );
    };
    if end > page_table_entries(space.htab_size_exp) {
        return lv1_answer(READ_HTAB_INDEX_R3);
    }
    let len = READ_HTAB_ENTRY_COUNT * PTE_BYTES as u64;
    let bytes = spaces
        .table_real(space.vas_id)
        .and_then(|table| ByteRange::new(GuestAddr::new(entry_address(table, index)), len))
        .and_then(|range| storage.read(range));
    let Some(bytes) = bytes else {
        return refuse(request, "a page table read with no storage for the table");
    };
    let mut outputs = [0u64; READ_HTAB_ENTRY_COUNT as usize + 1];
    let mut doubleword1 = [0u64; READ_HTAB_ENTRY_COUNT as usize];
    for (slot, entry) in bytes.chunks_exact(PTE_BYTES).enumerate() {
        outputs[slot] = read_u64(entry, 0);
        doubleword1[slot] = read_u64(entry, 8);
    }
    outputs[READ_HTAB_ENTRY_COUNT as usize] = read_htab_low_halfwords(doubleword1);
    HypercallDisposition::Answered(HypercallAnswer::lv1_status(
        zero_extended(LV1_SUCCESS),
        outputs,
    ))
}

/// The page that a written entry maps.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Page {
    /// The page's logical partition address.
    address: u64,
    /// Log2 of the page size.
    shift: u32,
    /// The large page bits that the stored entry carries.
    large_page_bits: u64,
}

/// The page that `doubleword1` maps in `space`.
///
/// `doubleword0` passed the handler's checks. The page's address is bits
/// 2:51 of doubleword 1. A large page takes
/// its size from the address space's page shifts, and its address loses
/// the large page bits [OFW-4.93 lv1 a:0x2D38D8-0x2D3938].
///
/// # Errors
///
/// Returns the r3 of lv1's worker for a large page that it refuses.
fn page_of(space: &AddressSpace, doubleword0: u64, doubleword1: u64) -> Result<Page, u64> {
    let address = doubleword1 & PTE_RPN;
    if !large(doubleword0) {
        return Ok(Page {
            address,
            shift: SMALL_PAGE_LOG2,
            large_page_bits: 0,
        });
    }
    let index = large_page_index(doubleword1);
    if u64::from(index) >= space.page_size_count {
        return Err(WRITE_HTAB_PAGE_SIZE_R3);
    }
    let shift = ((space.page_shifts >> (56 - 8 * index)) & 0xFF) as u32;
    let large_page_bits = large_page_bits(index);
    let address = address - large_page_bits;
    if address & ((1 << shift) - 1) != 0 {
        return Err(WRITE_HTAB_ALIGNMENT_R3);
    }
    Ok(Page {
        address,
        shift,
        large_page_bits,
    })
}

/// The r3 of the handler's checks of `doubleword0` and `doubleword1`, or `None` when both pass.
///
/// See [`WRITE_HTAB_INDEX_R3`] for the order.
fn doubleword_check(doubleword0: u64, doubleword1: u64) -> Option<u64> {
    let full_field = doubleword1 & LARGE_PAGE_FIELD == LARGE_PAGE_FIELD;
    if doubleword1 & WRITE_HTAB_DOUBLEWORD1_RESERVED != 0 || (full_field && large(doubleword0)) {
        Some(WRITE_HTAB_DOUBLEWORD1_R3)
    } else if doubleword0 & WRITE_HTAB_DOUBLEWORD0_RESERVED != 0 {
        Some(WRITE_HTAB_DOUBLEWORD0_R3)
    } else {
        None
    }
}

/// Doubleword 0 of each entry of the group from entry `first` of the
/// page table at the real address `table`, or `None` when no storage
/// holds the group.
fn group_doublewords0(storage: &GuestMemory, table: u64, first: u64) -> Option<Vec<(u64, u64)>> {
    let range = ByteRange::new(
        GuestAddr::new(entry_address(table, first)),
        PTEG_BYTES as u64,
    )?;
    let bytes = storage.read(range)?;
    Some(
        (first..)
            .zip(bytes.chunks_exact(PTE_BYTES))
            .map(|(index, entry)| (index, read_u64(entry, 0)))
            .collect(),
    )
}

/// Whether a valid `doubleword0` at entry `index` duplicates an entry of `space`'s table.
///
/// `table` is the real address of the table, or `None` when the table
/// has no storage. The value is `None` when no storage holds the two
/// groups. See [`duplicates`].
fn duplicate(
    storage: &GuestMemory,
    space: &AddressSpace,
    table: Option<u64>,
    index: u64,
    doubleword0: u64,
) -> Option<bool> {
    let table = table?;
    let own = index & !(PTES_PER_PTEG as u64 - 1);
    let other = other_group(index, space.htab_size_exp);
    let in_own = group_doublewords0(storage, table, own)?
        .into_iter()
        .any(|(slot, existing)| slot != index && duplicates(existing, doubleword0, true));
    let in_other = group_doublewords0(storage, table, other)?
        .into_iter()
        .any(|(_, existing)| duplicates(existing, doubleword0, false));
    Some(in_own || in_other)
}

/// Why a valid entry gets no real page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NoRealPage {
    /// lv1's translation of the page's region refuses the entry with this
    /// r3.
    Lv1(u64),
    /// CellGov refuses the entry for this reason.
    Refused(&'static str),
}

/// The partition's regions that a valid entry's page can lie in.
#[derive(Clone, Copy)]
struct Regions<'a> {
    spaces: &'a AddressSpaceState,
    spes: &'a LogicalSpeState,
    allocated: &'a MemoryState,
    device_bus: &'a DeviceBusState,
    real_mode_size: fn(&str) -> Option<u64>,
}

/// The real address of `page`, or why the entry gets none.
///
/// lv1 looks the logical partition address up in the partition's memory
/// regions and maps the page there [OFW-4.93 lv1 a:0x2D2510]
/// [OFW-4.93 lv1 a:0x2D30C4]. CellGov models five kinds of region:
/// - the mapping of a page table that `regions` holds, whose page lies in
///   the table's storage at the same offset;
/// - a region of a logical SPE, as [`region_page`] answers it for
///   `doubleword1`;
/// - a region that `allocate_memory` created, as
///   [`storage::region_page`] answers it;
/// - an MMIO mapping, as [`mmio_page::mapping_page`] answers it;
/// - the real mode region of `firmware`, which CellGov holds at real
///   address 0 below the page table storage, so the real address is the
///   logical partition address.
///
/// The page of an MMIO mapping lies in a device's MMIO, which no CellGov
/// storage holds. Each other page must lie in CellGov's storage. CellGov
/// refuses a page that lies in an MMIO mapping and in another region.
fn real_page(
    firmware: &str,
    regions: Regions<'_>,
    storage: &GuestMemory,
    page: Page,
    doubleword1: u64,
) -> Result<u64, NoRealPage> {
    let size = 1u64 << page.shift;
    let end = page.address.checked_add(size);
    // The inner value is `None` for a page in the mapping of a table with
    // no storage.
    let in_mapping = regions.spaces.spaces().iter().find_map(|space| {
        let mapping = space.mapping?;
        let size = htab_size(space.htab_size_exp);
        let inside = page.address >= mapping && end.is_some_and(|end| end <= mapping + size);
        let table = regions.spaces.table_real(space.vas_id);
        inside.then(|| table.map(|table| table + (page.address - mapping)))
    });
    let spe = region_page(regions.spes, page.address, page.shift, doubleword1);
    let allocated = storage::region_page(regions.allocated, page.address, page.shift);
    let mmio = mmio_page::mapping_page(firmware, regions.device_bus, page.address, page.shift);
    // lv1's region function never gives an address that a region of the
    // partition holds [OFW-4.93 lv1 a:0x2CEDE4-0x2CEDEC], and the region
    // of each address lies in the 2^e-byte block at that address. So no
    // two lv1 regions share a page. The MMIO map handler places each
    // mapping as lv1's allocator does [OFW-4.93 lv1 a:0x2CED84-0x2CED94],
    // so a page in two regions is a break of the model's own state.
    if mmio.is_some() && (in_mapping.is_some() || spe.is_some() || allocated.is_some()) {
        return Err(NoRealPage::Refused(
            "a valid entry whose page lies in an MMIO mapping and in another region",
        ));
    }
    let real = if let Some(real) = in_mapping {
        real.ok_or(NoRealPage::Refused(
            "a valid entry whose page lies in the mapping of a page table with no storage",
        ))?
    } else if let Some(answer) = spe {
        spe_real(answer)?
    } else if let Some(answer) = allocated {
        allocated_real(answer)?
    } else if let Some(answer) = mmio {
        return mmio_real(answer);
    } else {
        real_mode_page(regions.real_mode_size, firmware, page.address, end)?
    };
    if !storage.holds(real, size) || real > STORED_MEMORY_LIMIT {
        return Err(NoRealPage::Refused(
            "a valid entry whose page CellGov holds no storage for",
        ));
    }
    Ok(real)
}

/// The real address of a page in a region of a logical SPE, or why the
/// entry gets none.
fn spe_real(answer: SpeRegionPage) -> Result<u64, NoRealPage> {
    match answer {
        SpeRegionPage::Shadow(real) => Ok(real),
        SpeRegionPage::Lv1(r3) => Err(NoRealPage::Lv1(r3)),
        SpeRegionPage::Refused(what) => Err(NoRealPage::Refused(what)),
    }
}

/// The real address of a page in an allocated region, or why the entry
/// gets none.
fn allocated_real(answer: AllocatedPage) -> Result<u64, NoRealPage> {
    match answer {
        AllocatedPage::Stored(real) => Ok(real),
        AllocatedPage::Lv1(r3) => Err(NoRealPage::Lv1(r3)),
        AllocatedPage::NoStorage => Err(NoRealPage::Refused(
            "a valid entry whose page lies in an allocated memory region with no storage",
        )),
    }
}

/// The real address of a page in an MMIO mapping, or why the entry gets
/// none.
fn mmio_real(answer: MmioPage) -> Result<u64, NoRealPage> {
    match answer {
        MmioPage::Io(real) => Ok(real),
        MmioPage::Lv1(r3) => Err(NoRealPage::Lv1(r3)),
        MmioPage::Refused(what) => Err(NoRealPage::Refused(what)),
    }
}

/// The real address of a page at the logical partition address `address`
/// that ends at `end` in the real mode region, or the refusal.
fn real_mode_page(
    real_mode_size: fn(&str) -> Option<u64>,
    firmware: &str,
    address: u64,
    end: Option<u64>,
) -> Result<u64, NoRealPage> {
    let Some(rm_size) = real_mode_size(firmware) else {
        return Err(NoRealPage::Refused(
            "a valid entry at a firmware whose real mode size no source gives",
        ));
    };
    let limit = rm_size.min(PAGE_TABLE_STORAGE_BASE);
    if end.is_none_or(|end| end > limit) {
        return Err(NoRealPage::Refused(
            "a valid entry whose page lies outside the partition's real mode region",
        ));
    }
    Ok(address)
}

/// Hypercall 1: writes the entry at index r4 from doubleword 0 in r5 and doubleword 1 in r6.
///
/// The answer clears r4 to r10. The stored doubleword 1 holds the page's
/// real address and the WIMG that lv1 gives the address:
/// - M for an address at or below [`STORED_MEMORY_LIMIT`]; see
///   [`stored_doubleword1`];
/// - the WIMG of an I/O range for a higher address; see
///   [`stored_io_doubleword1`].
///
/// The answer writes the 16 bytes of the entry to the storage.
///
/// A call that fails one of lv1's checks gets lv1's own r3, with r4 to
/// r10 cleared. See [`WRITE_HTAB_INDEX_R3`] for the checks and their
/// order. lv1 checks the address space first, and CellGov answers it
/// only as [`named_space`] allows. lv1 translates the page of a valid
/// entry after the duplicate search, for these regions:
/// - a shadow page of a logical SPE in `spes`; see
///   [`cellgov_ps3_abi::lv1::spe`];
/// - a region in `allocated`; see
///   [`cellgov_ps3_abi::lv1::memory::WRITE_HTAB_ALLOCATED_PAGE_SIZE_R3`];
/// - an MMIO mapping in `device_bus`; see
///   [`cellgov_ps3_abi::lv1::device_bus::WRITE_HTAB_MMIO_PAGE_SIZE_R3`].
///
/// The call is a refusal in these cases:
/// - [`named_space`] refuses it;
/// - a valid entry that duplicates no entry in its group maps a page that
///   [`real_page`] refuses;
/// - no storage holds the entry's two groups.
pub(crate) fn write_entry(
    request: &HypercallRequest,
    firmware: Option<&str>,
    spaces: &AddressSpaceState,
    spes: &LogicalSpeState,
    allocated: &MemoryState,
    device_bus: &DeviceBusState,
    storage: &GuestMemory,
) -> HypercallDisposition {
    let regions = Regions {
        spaces,
        spes,
        allocated,
        device_bus,
        real_mode_size: memory::real_mode_size,
    };
    write_entry_with(request, firmware, regions, storage)
}

/// The answer of [`write_entry`], with the partition's regions and the source of the real mode
/// size in `regions`.
fn write_entry_with(
    request: &HypercallRequest,
    firmware: Option<&str>,
    regions: Regions<'_>,
    storage: &GuestMemory,
) -> HypercallDisposition {
    let space = match named_space(request, firmware, regions.spaces) {
        Ok(space) => space,
        Err(missing) => return missing.disposition(request),
    };
    let (index, doubleword0, doubleword1) = (request.r(4), request.r(5), request.r(6));
    if index >= page_table_entries(space.htab_size_exp) {
        return lv1_answer(WRITE_HTAB_INDEX_R3);
    }
    if let Some(r3) = doubleword_check(doubleword0, doubleword1) {
        return lv1_answer(r3);
    }
    let page = match page_of(space, doubleword0, doubleword1) {
        Ok(page) => page,
        Err(r3) => return lv1_answer(r3),
    };
    let table = regions.spaces.table_real(space.vas_id);
    let mut real = 0;
    if valid(doubleword0) {
        // named_space refuses a call with no firmware version.
        let firmware = firmware.unwrap_or_default();
        // lv1 searches the group before it translates the page
        // [OFW-4.93 lv1 a:0x2D2F20-0x2D3038], so a duplicate answers before
        // any translation result, a CellGov refusal included.
        let translated = real_page(firmware, regions, storage, page, doubleword1);
        match duplicate(storage, space, table, index, doubleword0) {
            Some(true) => return lv1_answer(WRITE_HTAB_DUPLICATE_R3),
            Some(false) => {}
            None => return refuse(request, "a page table write with no storage for the table"),
        }
        real = match translated {
            Ok(real) => real,
            Err(NoRealPage::Lv1(r3)) => return lv1_answer(r3),
            Err(NoRealPage::Refused(what)) => return refuse(request, what),
        };
    }
    // lv1's writer picks the WIMG by the real address
    // [OFW-4.93 lv1 a:0x2D3114].
    let stored = if real > STORED_MEMORY_LIMIT {
        stored_io_doubleword1(doubleword1, real, page.large_page_bits)
    } else {
        stored_doubleword1(doubleword1, real, page.large_page_bits)
    };
    let mut bytes = doubleword0.to_be_bytes().to_vec();
    bytes.extend_from_slice(&stored.to_be_bytes());
    let Some(range) = table
        .and_then(|table| {
            ByteRange::new(
                GuestAddr::new(entry_address(table, index)),
                PTE_BYTES as u64,
            )
        })
        .filter(|range| storage.holds(range.start().raw(), range.length()))
    else {
        return refuse(request, "a page table write with no storage for the table");
    };
    let mut answer = HypercallAnswer::lv1_status(zero_extended(LV1_SUCCESS), []);
    answer.effects.push(Effect::SharedWriteIntent {
        range,
        bytes: WritePayload::new(bytes),
        source: request.unit,
        source_time: request.tick,
    });
    HypercallDisposition::Answered(answer)
}

/// Hypercall 122: maps the page table of the address space that r3 names.
///
/// The answer gives the mapping's logical partition address in r4. The
/// mapping takes the next value of the partition's region counter in
/// `memory`, and skips each address in `held`, as an allocation does; see
/// [`place`]. Its address encodes the page table's size exponent. The
/// answer carries the mapping, and the hypervisor keeps it. r5 to r10
/// get 0: lv1 leaves data from its own stack there, and CellGov does not
/// model that data.
///
/// The call is a refusal in these cases:
/// - [`named_space`] refuses it;
/// - the page table is already mapped, and no kernel call is shown to map
///   it twice;
/// - no source gives the region counter for `firmware`;
/// - [`place`] gives no address.
pub(crate) fn map_table(
    request: &HypercallRequest,
    firmware: Option<&str>,
    spaces: &AddressSpaceState,
    memory: &MemoryState,
    held: &HeldRegions,
) -> HypercallDisposition {
    let space = match named_space(request, firmware, spaces) {
        Ok(space) => space,
        Err(missing) => return missing.disposition(request),
    };
    if space.mapping.is_some() {
        return refuse(request, "a second mapping of a page table");
    }
    // named_space refuses a call with no firmware version.
    let firmware = firmware.unwrap_or_default();
    let Some(boot) = game_os_region_counter_at_boot(firmware) else {
        return refuse(
            request,
            "a page table mapping at a firmware whose region counter no source gives",
        );
    };
    let first = memory.next_counter().unwrap_or(boot);
    let (lpar_address, counter) = match place(htab_size(space.htab_size_exp), first, boot, held) {
        Ok(placement) => (placement.address, placement.counter),
        Err(missing) => return refuse(request, missing.what()),
    };
    let mut answer = HypercallAnswer::lv1_status(zero_extended(LV1_SUCCESS), [lpar_address]);
    answer.push_change(AddressSpaceChange::Mapped {
        vas_id: space.vas_id,
        lpar_address,
        counter,
    });
    HypercallDisposition::Answered(answer)
}

#[cfg(test)]
#[path = "tests/page_table_map_tests.rs"]
mod map_tests;

#[cfg(test)]
#[path = "tests/page_table_read_tests.rs"]
mod read_tests;

#[cfg(test)]
#[path = "tests/page_table_write_tests.rs"]
mod write_tests;

#[cfg(test)]
#[path = "tests/page_table_vas_tests.rs"]
mod vas_tests;

#[cfg(test)]
#[path = "tests/page_table_spe_tests.rs"]
mod spe_tests;

#[cfg(test)]
#[path = "tests/page_table_allocated_tests.rs"]
mod allocated_tests;

#[cfg(test)]
#[path = "tests/page_table_boot_region_tests.rs"]
mod boot_region_tests;

#[cfg(test)]
#[path = "tests/page_table_mmio_tests.rs"]
mod mmio_tests;

#[cfg(test)]
#[path = "tests/page_table_mmio_shift_tests.rs"]
mod mmio_shift_tests;

#[cfg(test)]
#[path = "tests/page_table_mmio_bounds_tests.rs"]
mod mmio_bounds_tests;

#[cfg(test)]
#[path = "tests/page_table_real_mode_size_tests.rs"]
mod real_mode_size_tests;

#[cfg(test)]
#[path = "tests/page_table_storage_tests.rs"]
mod storage_tests;
