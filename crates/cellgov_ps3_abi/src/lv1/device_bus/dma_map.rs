//! The facts of `map_device_dma_region`: the checks, the regions of lv1's
//! own partition, and the mapping of the GameOS kernel's boot.
//!
//! # `map_device_dma_region`
//!
//! The handler [OFW-4.93 lv1 a:0x306770] keeps r3 to r8
//! [OFW-4.93 lv1 a:0x306784-0x3067B8] and does these steps:
//! 1. It refuses a bus id above [`OPEN_DEVICE_BUS_MAX`] with
//!    [`DMA_MAP_BUS_RANGE_R3`] [OFW-4.93 lv1 a:0x306818-0x30681C]
//!    [OFW-4.93 lv1 a:0x306904-0x306914].
//! 2. It refuses flags that set a bit outside [`DMA_MAP_FLAGS_MASK`] with
//!    [`DMA_MAP_FLAGS_R3`] [OFW-4.93 lv1 a:0x306820-0x30683C].
//! 3. It calls the map function [OFW-4.93 lv1 a:0x236D1C] with the
//!    caller's partition and r3 to r8 [OFW-4.93 lv1 a:0x3068D0-0x3068F8].
//! 4. It returns that function's r3 [OFW-4.93 lv1 a:0x3068FC]
//!    [OFW-4.93 lv1 a:0x3068A4]. It writes no return slot, and the
//!    vector's entry zeroes the slots [OFW-4.93 lv1 a:0x0C08]. So r4 to
//!    r10 return 0.
//!
//! The map function does these steps. The first failure is the answer:
//! 1. The access check [OFW-4.93 lv1 a:0x2363E0]
//!    [OFW-4.93 lv1 a:0x236D74], which `allocate_device_dma_region` also
//!    calls. Bus 0 gets [`DMA_MAP_NO_BUS_R3`]. A partition that did not
//!    open the device gets [`DMA_MAP_NOT_OPEN_R3`]. The function
//!    zero-extends the status and sets bit (mark + 31) for a mark other
//!    than 0 [OFW-4.93 lv1 a:0x236D88] [OFW-4.93 lv1 a:0x236E1C-0x236E38].
//! 2. The device lookup with the bus id and the device id
//!    [OFW-4.93 lv1 a:0x236D80-0x236D90]. The section on devices 1 and 2
//!    below gives the device that it finds for bus 1. The function takes
//!    no path of its own for devices 1 and 2 of bus 1
//!    [OFW-4.93 lv1 a:0x236D1C-0x236DB8].
//! 3. The device's map function [OFW-4.93 lv1 a:0x227218], with the
//!    device, the partition, the bus address, the logical partition
//!    address, the length, the flags and a mark
//!    [OFW-4.93 lv1 a:0x236D94-0x236DB8]. A nonzero r3 gets the treatment
//!    of step 1 [OFW-4.93 lv1 a:0x236DBC-0x236DE4].
//!
//! The device's map function holds the device's lock at +88
//! [OFW-4.93 lv1 a:0x227268-0x22728C] and does these steps:
//! 1. It finds the region that holds the bus address with the device's
//!    entry at +200 [OFW-4.93 lv1 a:0x227290-0x2272B8]
//!    [OFW-4.93 lv1 a:0x35C940-0x35C944] [OFW-4.93 lv1 a:0x32D450-0x32D454].
//!    That entry walks the device's region list at +96. It checks no
//!    partition [OFW-4.93 lv1 a:0x22799C-0x2279D0]. When no region holds
//!    the address, the answer is [`DMA_MAP_NO_REGION_R3`]
//!    [OFW-4.93 lv1 a:0x227444-0x22744C].
//! 2. A length of 0 or above the region's size gets
//!    [`DMA_MAP_LENGTH_R3`] [OFW-4.93 lv1 a:0x2272BC-0x2272CC]
//!    [OFW-4.93 lv1 a:0x2273E8-0x2273F0].
//! 3. The range check [OFW-4.93 lv1 a:0x2257C4-0x225848]; see
//!    [`dma_map_range_free`]. A range that fails it gets
//!    [`DMA_MAP_BUSY_R3`] [OFW-4.93 lv1 a:0x2272D0-0x2272E4]
//!    [OFW-4.93 lv1 a:0x2273F8-0x227400].
//! 4. The page holds 2^(page shift) bytes of the region
//!    [OFW-4.93 lv1 a:0x2272E8-0x227328]. The checks of
//!    [`dma_map_alignment_r3`] run next.
//! 5. It clears the mark and calls the device's entry at +168
//!    [OFW-4.93 lv1 a:0x22735C-0x227398] [OFW-4.93 lv1 a:0x35C920-0x35C924]
//!    [OFW-4.93 lv1 a:0x32D480-0x32D484]. It returns that entry's r3
//!    sign-extended [OFW-4.93 lv1 a:0x2273B0].
//!
//! Device 0's entry at +168 [OFW-4.93 lv1 a:0x227B34] does these steps:
//! 1. It calls the device's entry at +120, which gives the I/O address of
//!    the bus address [OFW-4.93 lv1 a:0x227BDC-0x227C10]
//!    [OFW-4.93 lv1 a:0x35C8F0-0x35C8F4] [OFW-4.93 lv1 a:0x32E008-0x32E00C].
//!    That entry returns -17 in these cases
//!    [OFW-4.93 lv1 a:0x22D718-0x22D760]:
//!    - the 32-bit word at +160 of the device is 16;
//!    - the bus address is outside [`DMA_MAP_IO_BUS_ADDRESSES`].
//!
//!    The capture [`DMA_REGION_CAPTURE`] reads 2 in that word.
//! 2. It calls the device's entry at +136
//!    [OFW-4.93 lv1 a:0x227C50-0x227C98] [OFW-4.93 lv1 a:0x35C900-0x35C904]
//!    [OFW-4.93 lv1 a:0x32D360-0x32D364]. That entry writes the IOPTE of
//!    each page with an lv1 function [OFW-4.93 lv1 a:0x2D7E9C], and
//!    returns -17 when one write fails [OFW-4.93 lv1 a:0x227124-0x2271E4].
//!    The writes read lv1's IOMMU state, which CellGov does not model.
//! 3. It makes the mapping with the mapping function
//!    [OFW-4.93 lv1 a:0x225E18] [OFW-4.93 lv1 a:0x227C9C]. That function
//!    repeats the range check [OFW-4.93 lv1 a:0x225F7C-0x226048], so only
//!    its 96-byte allocation can fail [OFW-4.93 lv1 a:0x225E60-0x225E74].
//!    On that failure, the entry undoes the writes and returns
//!    `LV1_RESOURCE_SHORTAGE` (-2) [OFW-4.93 lv1 a:0x227D08-0x227D2C]. No
//!    input of the call selects that case.
//! 4. It stores the logical partition address at +80 of the mapping and
//!    returns 0 [OFW-4.93 lv1 a:0x227CC4-0x227CD0].
//!
//! The mapping function fills the mapping with the partition, the offset
//! of the bus address in the region, the logical partition address and
//! the length. It does not keep the flags
//! [OFW-4.93 lv1 a:0x225E78-0x225EB4]. It links the mapping into the
//! region's list at +72 [OFW-4.93 lv1 a:0x225EB8-0x225F50].
//!
//! # `map_device_dma_region` for devices 1 and 2 of bus 1
//!
//! The device lookup sends bus 1 to the lookup of bus 1
//! [OFW-4.93 lv1 a:0x236594-0x2365AC] [OFW-4.93 lv1 a:0x236648-0x236658].
//! That lookup takes slot 0, 3 or 4 of the system bus object for device
//! 0, 1 or 2 [OFW-4.93 lv1 a:0x235D88-0x235DE8]
//! [OFW-4.93 lv1 a:0x22B838-0x22B844]. These are the devices of
//! [`DMA_MAP_DEVICES`]. The map function then calls the device's map
//! function with that device, as for device 0.
//!
//! The device's map function gets the device and no device id
//! [OFW-4.93 lv1 a:0x236D94-0x236DB8]. So its checks and the entries
//! that it calls come from the device and its method table. The captures
//! read these facts of devices 1 and 2:
//! - The method table is device 0's ([`DMA_SLOTS_CAPTURE`] and
//!   [`MMIO_DEVICE_2_CAPTURE`]), so the checks and the entries are device
//!   0's.
//! - The region list holds only the regions of the game OS partition, so
//!   lv1 holds no region of its own for the device.
//! - The 32-bit word at +160 is 8 for device 1 and 9 for device 2
//!   ([`DMA_MAPPINGS_CAPTURE`]). So the entry at +120 does not return
//!   -17 for the word; see [`DMA_MAP_NO_IO_WORD`].
//!
//! For a bus address in [`DMA_MAP_IO_BUS_ADDRESSES`], the entry at +120
//! gives an I/O address. That address is the low 31 bits of the bus
//! address plus the device's doublewords at +168 and +176
//! [OFW-4.93 lv1 a:0x22D75C-0x22D778]. The capture
//! [`DMA_MAPPINGS_CAPTURE`] reads 0x400000000 and 0 for device 1, and
//! 0x500000000 and 0 for device 2. The IOPTE writes for these I/O
//! addresses read lv1's IOMMU state, which CellGov does not model. The
//! capture holds the mapping of each of the kernel's calls, so lv1's
//! writes succeeded for them.
//!
//! # The 4.93 kernel's call of `map_device_dma_region`
//!
//! The kernel's caller gives its wrapper 0 as the flags word
//! [OFW-4.93 lv2_kernel a:0x80000000000954a4-0x80000000000954d4]. So the
//! wrapper sets the IOPTE flags to 0xE000000000000000
//! [OFW-4.93 lv2_kernel a:0x800000000007d50c-0x800000000007d5bc]. For
//! each page of its buffer, the wrapper sends the call with these inputs
//! [OFW-4.93 lv2_kernel a:0x800000000007d678-0x800000000007d6b4]:
//! - r3 and r4: the bus id and the device id;
//! - r5: the logical partition address of the page;
//! - r6: the bus address of the page;
//! - r7: the page size;
//! - r8: the flags.
//!
//! A status of -9 or -2 makes the wrapper unmap the pages that it mapped
//! [OFW-4.93 lv2_kernel a:0x800000000007d6c4-0x800000000007d73c]. The
//! captures [`DMA_MAP_CAPTURE`] and [`DMA_MAPPINGS_CAPTURE`] give lv1's
//! record of the calls; see [`GAME_OS_DMA_MAPPINGS`].
//!
//! [`DMA_REGION_CAPTURE`]: super::DMA_REGION_CAPTURE
//! [`DMA_SLOTS_CAPTURE`]: super::DMA_SLOTS_CAPTURE
//! [`MMIO_DEVICE_2_CAPTURE`]: super::MMIO_DEVICE_2_CAPTURE
//! [`OPEN_DEVICE_BUS_MAX`]: super::OPEN_DEVICE_BUS_MAX

use std::ops::RangeInclusive;

use crate::lv1::status::{zero_extended, LV1_WRONG_STATE};

/// The handler of hypercall 176 in one firmware's lv1.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DmaMapHandler {
    /// The firmware version, such as `"4.93"`.
    pub firmware: &'static str,
    /// The address of the `map_device_dma_region` handler.
    pub handler: u32,
}

/// Each firmware whose lv1 image shows the handler, ascending.
///
/// The address is the entry of hypercall 176 in the live 4.93 hypercall
/// table ([`crate::lv1::table::LIVE_TABLE_HANDLERS`]).
pub const DMA_MAP_HANDLERS: &[DmaMapHandler] = &[DmaMapHandler {
    firmware: "4.93",
    handler: 0x306770,
}];

/// The handler of the firmware version `firmware`, or `None` when no lv1
/// image read shows it.
pub fn dma_map_handler(firmware: &str) -> Option<&'static DmaMapHandler> {
    DMA_MAP_HANDLERS
        .iter()
        .find(|handler| handler.firmware == firmware)
}

/// The r3 of `map_device_dma_region` for a bus id above
/// [`OPEN_DEVICE_BUS_MAX`]: `LV1_ILLEGAL_PARAMETER_VALUE` (-17), with r3
/// marked in the high word [OFW-4.93 lv1 a:0x306904-0x306910].
///
/// Bit 32 marks r3, bit 33 r4, bit 34 r5, bit 35 r6, bit 36 r7 and bit
/// 37 r8.
///
/// [`OPEN_DEVICE_BUS_MAX`]: super::OPEN_DEVICE_BUS_MAX
pub const DMA_MAP_BUS_RANGE_R3: u64 = 0x0000_0001_FFFF_FFEF;

/// The IOPTE flags that the handler passes: bits 0 to 4 and bit 52, in
/// the bit order in which bit 0 is the most significant
/// [OFW-4.93 lv1 a:0x306820-0x30682C].
pub const DMA_MAP_FLAGS_MASK: u64 = 0xF800_0000_0000_0800;

/// The r3 for flags that set a bit outside [`DMA_MAP_FLAGS_MASK`]:
/// `LV1_ILLEGAL_PARAMETER_VALUE` (-17), with r8 marked
/// [OFW-4.93 lv1 a:0x306830-0x30683C].
pub const DMA_MAP_FLAGS_R3: u64 = 0x0000_0020_FFFF_FFEF;

/// The r3 for bus id 0: `LV1_NO_ENTRY` (-6), with r3 marked
/// [OFW-4.93 lv1 a:0x236420-0x236428] [OFW-4.93 lv1 a:0x236E1C-0x236E38].
pub const DMA_MAP_NO_BUS_R3: u64 = 0x0000_0001_FFFF_FFFA;

/// The r3 when the calling partition did not open the device:
/// `LV1_WRONG_STATE` (-11), zero-extended
/// [OFW-4.93 lv1 a:0x236534-0x236538] [OFW-4.93 lv1 a:0x236D88].
pub const DMA_MAP_NOT_OPEN_R3: u64 = zero_extended(LV1_WRONG_STATE);

/// The r3 when no region of the device holds the bus address:
/// `LV1_NO_ENTRY` (-6), with r6 marked
/// [OFW-4.93 lv1 a:0x227444-0x22744C] [OFW-4.93 lv1 a:0x236DC8-0x236DE4].
pub const DMA_MAP_NO_REGION_R3: u64 = 0x0000_0008_FFFF_FFFA;

/// The r3 for a length of 0, a length above the region's size, or a
/// length that is not a multiple of the page:
/// `LV1_ILLEGAL_PARAMETER_VALUE` (-17), with r7 marked
/// [OFW-4.93 lv1 a:0x2273E8-0x2273F0] [OFW-4.93 lv1 a:0x236DC8-0x236DE4].
pub const DMA_MAP_LENGTH_R3: u64 = 0x0000_0010_FFFF_FFEF;

/// The r3 for a range that fails [`dma_map_range_free`]: `LV1_BUSY`
/// (-9), with r6 marked [OFW-4.93 lv1 a:0x2273F8-0x227400]
/// [OFW-4.93 lv1 a:0x236DC8-0x236DE4].
pub const DMA_MAP_BUSY_R3: u64 = 0x0000_0008_FFFF_FFF7;

/// The r3 for a bus address that is not a multiple of the page:
/// `LV1_ILLEGAL_PARAMETER_VALUE` (-17), with r6 marked
/// [OFW-4.93 lv1 a:0x227408-0x227410] [OFW-4.93 lv1 a:0x236DC8-0x236DE4].
pub const DMA_MAP_BUS_ALIGNMENT_R3: u64 = 0x0000_0008_FFFF_FFEF;

/// The r3 for a logical partition address that is not a multiple of the
/// page: `LV1_ILLEGAL_PARAMETER_VALUE` (-17), with r5 marked
/// [OFW-4.93 lv1 a:0x227418-0x227420] [OFW-4.93 lv1 a:0x236DC8-0x236DE4].
pub const DMA_MAP_LPAR_ALIGNMENT_R3: u64 = 0x0000_0004_FFFF_FFEF;

/// The bus addresses to which the device's entry at +120 gives an I/O
/// address [OFW-4.93 lv1 a:0x22D71C-0x22D760].
pub const DMA_MAP_IO_BUS_ADDRESSES: RangeInclusive<u64> = 0x8000_0000..=0xFFFF_FFFF;

/// The value of the device's 32-bit word at +160 for which the entry at
/// +120 gives no I/O address and returns -17
/// [OFW-4.93 lv1 a:0x22D718-0x22D738].
pub const DMA_MAP_NO_IO_WORD: u32 = 16;

/// The devices of bus 1 whose map path the lookup of bus 1 reaches:
/// devices 0, 1 and 2 [OFW-4.93 lv1 a:0x235D98-0x235DB8].
///
/// Device 3 takes a path of its own [OFW-4.93 lv1 a:0x235DBC-0x235DC0].
/// Any other device id takes slot 14 of the system bus object, the
/// doubleword at +144 [OFW-4.93 lv1 a:0x235DC4-0x235DE8]
/// [OFW-4.93 lv1 a:0x22B838-0x22B844]. No capture reads the device that
/// either path gives.
pub const DMA_MAP_DEVICES: [u64; 3] = [0, 1, 2];

/// Whether the range check passes the `length` bytes at `bus_address`
/// in the region of `region_size` bytes at `region_address`.
///
/// `mappings` gives the offset in the region and the length of each
/// mapping of the region. The check fails in these cases
/// [OFW-4.93 lv1 a:0x2257C4-0x225848]:
/// - the bus address is below the region;
/// - the end of the range is past the end of the region;
/// - the range overlaps a mapping.
///
/// lv1 adds and compares each value at 64 bits, unsigned.
pub fn dma_map_range_free(
    region_address: u64,
    region_size: u64,
    mappings: impl IntoIterator<Item = (u64, u64)>,
    bus_address: u64,
    length: u64,
) -> bool {
    if region_address > bus_address
        || bus_address.wrapping_add(length) > region_address.wrapping_add(region_size)
    {
        return false;
    }
    if length == 0 {
        return true;
    }
    let offset = bus_address - region_address;
    let end = offset.wrapping_add(length);
    mappings.into_iter().all(|(start, len)| {
        if offset >= start {
            start.wrapping_add(len) <= offset
        } else {
            end <= start
        }
    })
}

/// The r3 of the page checks for `bus_address`, `lpar_address` and
/// `length` in a region whose pages hold 2^`page_shift` bytes, or `None`
/// when they pass.
///
/// The checks run in this order
/// [OFW-4.93 lv1 a:0x22732C-0x227358]:
/// 1. a bus address that is not a multiple of the page gets
///    [`DMA_MAP_BUS_ALIGNMENT_R3`];
/// 2. a logical partition address that is not a multiple of the page
///    gets [`DMA_MAP_LPAR_ALIGNMENT_R3`];
/// 3. a length that is not a multiple of the page gets
///    [`DMA_MAP_LENGTH_R3`].
///
/// The page shift is one of [`DMA_REGION_PAGE_SHIFTS`], because the
/// allocate function accepts no other.
///
/// [`DMA_REGION_PAGE_SHIFTS`]: super::DMA_REGION_PAGE_SHIFTS
pub const fn dma_map_alignment_r3(
    bus_address: u64,
    lpar_address: u64,
    length: u64,
    page_shift: u64,
) -> Option<u64> {
    let page = 1u64 << page_shift;
    if !bus_address.is_multiple_of(page) {
        Some(DMA_MAP_BUS_ALIGNMENT_R3)
    } else if !lpar_address.is_multiple_of(page) {
        Some(DMA_MAP_LPAR_ALIGNMENT_R3)
    } else if !length.is_multiple_of(page) {
        Some(DMA_MAP_LENGTH_R3)
    } else {
        None
    }
}

/// A DMA region that lv1 holds for itself in one firmware.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Lv1DmaRegion {
    /// The firmware version, such as `"4.93"`.
    pub firmware: &'static str,
    /// The bus id of the region's device.
    pub bus: u64,
    /// The device id of the region's device.
    pub device: u64,
    /// The bus address of the region.
    pub address: u64,
    /// The size of the region, in bytes.
    pub size: u64,
    /// The page shift of the region.
    pub page_shift: u64,
}

/// Each region of lv1's own partition, by firmware.
///
/// The 4.93 row is region 1 of the capture [`DMA_REGION_CAPTURE`], which
/// holds partition 0 at +8.
///
/// [`DMA_REGION_CAPTURE`]: super::DMA_REGION_CAPTURE
pub const LV1_DMA_REGIONS: &[Lv1DmaRegion] = &[Lv1DmaRegion {
    firmware: "4.93",
    bus: 1,
    device: 0,
    address: 0xA000_0000,
    size: 0x8000,
    page_shift: 12,
}];

/// The region of lv1's own partition that holds `bus_address` for device
/// `device` of bus `bus`, or `None` when no capture gives one.
pub fn lv1_dma_region(
    firmware: &str,
    bus: u64,
    device: u64,
    bus_address: u64,
) -> Option<&'static Lv1DmaRegion> {
    LV1_DMA_REGIONS.iter().find(|region| {
        region.firmware == firmware
            && region.bus == bus
            && region.device == device
            && region.address <= bus_address
            && bus_address - region.address < region.size
    })
}

/// The mapping that the console's lv1 made for one call of the GameOS
/// kernel's boot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GameOsDmaMapping {
    /// The firmware version, such as `"4.93"`.
    pub firmware: &'static str,
    /// The bus id of the call.
    pub bus: u64,
    /// The device id of the call.
    pub device: u64,
    /// The bus address of the call.
    pub bus_address: u64,
    /// The logical partition address of the call.
    pub lpar_address: u64,
    /// The length of the call, in bytes.
    pub length: u64,
}

/// Each mapping of a kernel's boot calls, by firmware, in the order of
/// the calls.
///
/// The 4.93 rows are these:
/// - The one mapping of the network device's region at 0xC0000000. The
///   capture [`DMA_MAP_CAPTURE`] finds it in the region's list, with the
///   offset 0 at +40, the address at +48 and the length at +56.
/// - The 64 KB pages of devices 1 and 2: seven pages in the region at
///   0xC0080000 of each device, then two pages in the region at
///   0xC0000000 of each device. The capture [`DMA_MAPPINGS_CAPTURE`] finds
///   each of them in its region's list, with the same fields.
///
/// The mapping function keeps each region's list sorted by the offset at
/// +16, highest first [OFW-4.93 lv1 a:0x225EB8-0x225F10]
/// [OFW-4.93 lv1 a:0x225FBC-0x225FE0]. It writes that offset and the
/// mapping's class [OFW-4.93 lv1 a:0x225E80-0x225EA4]. The entry at +16
/// of that class [OFW-4.93 lv1 a:0x35C548-0x35C54C]
/// [OFW-4.93 lv1 a:0x32D108-0x32D10C] is the compare function
/// [OFW-4.93 lv1 a:0x225C88-0x225CA8]. So the order of a list gives no
/// order of the calls.
pub const GAME_OS_DMA_MAPPINGS: &[GameOsDmaMapping] = &[
    GameOsDmaMapping {
        firmware: "4.93",
        bus: 1,
        device: 0,
        bus_address: 0xC000_0000,
        lpar_address: 0x80_0000,
        length: 0x10_0000,
    },
    kernel_page(1, 0xC008_0000, 0x5000_0032_0000),
    kernel_page(1, 0xC009_0000, 0x5000_0033_0000),
    kernel_page(1, 0xC00A_0000, 0x5000_0034_0000),
    kernel_page(1, 0xC00B_0000, 0x5000_0035_0000),
    kernel_page(1, 0xC00C_0000, 0x5000_0036_0000),
    kernel_page(1, 0xC00D_0000, 0x5000_0037_0000),
    kernel_page(1, 0xC00E_0000, 0x90_0000),
    kernel_page(2, 0xC008_0000, 0x5000_0032_0000),
    kernel_page(2, 0xC009_0000, 0x5000_0033_0000),
    kernel_page(2, 0xC00A_0000, 0x5000_0034_0000),
    kernel_page(2, 0xC00B_0000, 0x5000_0035_0000),
    kernel_page(2, 0xC00C_0000, 0x5000_0036_0000),
    kernel_page(2, 0xC00D_0000, 0x5000_0037_0000),
    kernel_page(2, 0xC00E_0000, 0x90_0000),
    kernel_page(1, 0xC000_0000, 0x92_0000),
    kernel_page(1, 0xC001_0000, 0x93_0000),
    kernel_page(2, 0xC000_0000, 0x92_0000),
    kernel_page(2, 0xC001_0000, 0x93_0000),
];

/// The length of each page that the 4.93 kernel maps for devices 1 and 2.
const KERNEL_PAGE: u64 = 0x1_0000;

/// The 4.93 mapping of one 64 KB page of device `device` of bus 1.
const fn kernel_page(device: u64, bus_address: u64, lpar_address: u64) -> GameOsDmaMapping {
    GameOsDmaMapping {
        firmware: "4.93",
        bus: 1,
        device,
        bus_address,
        lpar_address,
        length: KERNEL_PAGE,
    }
}

/// The IOPTE flags of each call that [`GAME_OS_DMA_MAPPINGS`] holds.
///
/// The kernel's wrapper builds them from a flags word of 0
/// [OFW-4.93 lv2_kernel a:0x800000000007d50c-0x800000000007d5bc], and
/// CellGov's boot of the 4.93 kernel sends each of the rows' calls with
/// them.
pub const GAME_OS_DMA_MAP_FLAGS: u64 = 0xE000_0000_0000_0000;

/// Whether the call at `firmware` with these inputs is a row of
/// [`GAME_OS_DMA_MAPPINGS`] sent with [`GAME_OS_DMA_MAP_FLAGS`].
///
/// A console capture holds the mapping that lv1 made for each row. The
/// IOPTE writes of any other call read lv1's IOMMU state, which no image
/// read or console capture gives.
pub fn is_game_os_dma_mapping(
    firmware: &str,
    bus: u64,
    device: u64,
    bus_address: u64,
    lpar_address: u64,
    length: u64,
    flags: u64,
) -> bool {
    flags == GAME_OS_DMA_MAP_FLAGS
        && GAME_OS_DMA_MAPPINGS.iter().any(|row| {
            row.firmware == firmware
                && (
                    row.bus,
                    row.device,
                    row.bus_address,
                    row.lpar_address,
                    row.length,
                ) == (bus, device, bus_address, lpar_address, length)
        })
}

/// The console capture of the mapping that lv1 holds in the network
/// device's game OS region.
///
/// The path is relative to the workspace root. Its README gives the
/// method and the analysis.
pub const DMA_MAP_CAPTURE: &str = "tests/micro/lv1_device_dma_map";

/// The console capture of the mappings that lv1 holds in the DMA regions
/// of devices 1 and 2 of bus 1.
///
/// The path is relative to the workspace root. Its README gives the
/// method and the analysis.
pub const DMA_MAPPINGS_CAPTURE: &str = "tests/micro/lv1_device_dma_mappings";
