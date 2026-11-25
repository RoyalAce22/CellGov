//! The numbers of the memory and page table hypercalls.

/// `allocate_memory`: create a memory region in the caller's partition.
///
/// The inputs:
/// - r3 is the region size, a multiple of the page size.
/// - r4 is the page size exponent.
/// - r5 is 0.
/// - r6 holds the flags.
///
/// On success, r4 holds the region's logical partition address and r5
/// its memory unit id. Linux `arch/powerpc/platforms/ps3/mm.c`,
/// `ps3_mm_region_create`, makes the call in this form and keeps r4 as
/// the region base. [PS3DEVWIKI-HV-SYSCALL-REF s:lv1_allocate_memory]
/// gives the same registers.
pub const ALLOCATE_MEMORY: u64 = 0;

/// `write_htab_entry`: write one entry of an address space's page table.
///
/// The inputs:
/// - r3 is the vas id, or 0 for the caller's address space.
/// - r4 is the entry index.
/// - r5 is doubleword 0 of the entry.
/// - r6 is doubleword 1, with the page's logical partition address.
///
/// There is no output. The lv1 handler [OFW-4.93 lv1 a:0x2ED1C0] reads the
/// inputs in this form; see [`crate::lv1::page_table`].
pub const WRITE_HTAB_ENTRY: u64 = 1;

/// `construct_virtual_address_space`: create a virtual address space
/// with its own hashed page table.
///
/// The inputs:
/// - r3 is the page table size exponent.
/// - r4 is the number of page sizes.
/// - r5 holds the page shifts, 8 bits each from the most significant
///   byte.
///
/// On success, r4 holds the vas id and r5 the page table size in bytes.
/// Linux `arch/powerpc/platforms/ps3/mm.c`, `ps3_mm_vas_create`, makes
/// the call in this form.
/// [PS3DEVWIKI-HV-SYSCALL-REF s:lv1_construct_virtual_address_space]
/// gives the same registers.
pub const CONSTRUCT_VIRTUAL_ADDRESS_SPACE: u64 = 2;

/// `select_virtual_address_space`: the vas id in r3, and no output.
/// Linux `ps3_mm_vas_create` makes the call with the id that
/// [`CONSTRUCT_VIRTUAL_ADDRESS_SPACE`] returned.
pub const SELECT_VIRTUAL_ADDRESS_SPACE: u64 = 7;

/// `read_htab_entries`: read four entries of an address space's page
/// table.
///
/// The inputs are the vas id in r3, or 0 for the caller's address space,
/// and the index of the first entry in r4. On success, r4 to r7 hold
/// doubleword 0 of each entry, and r8 holds the low halfword of
/// doubleword 1 of each entry. The lv1 handler [OFW-4.93 lv1 a:0x2ECAF8]
/// reads the inputs and writes the outputs in this form; see
/// [`crate::lv1::page_table`].
pub const READ_HTAB_ENTRIES: u64 = 95;

/// `map_htab`: map an address space's page table into the partition's
/// logical address space.
///
/// The input is the vas id in r3, or 0 for the caller's address space.
/// On success, r4 holds the mapping's logical partition address. The lv1
/// worker [OFW-4.93 lv1 a:0x2DDB20] reads the input and writes the output
/// in this form; see [`crate::lv1::page_table::MAP_HTAB_RESOURCE_SHORTAGE_R3`].
pub const MAP_HTAB: u64 = 122;
