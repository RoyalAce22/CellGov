//! The MMIO regions that lv1's init gives devices 1 and 2 of bus 1, and
//! the real address of each region.

/// An MMIO region that lv1's init gives a device in one firmware.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MmioRegion {
    /// The firmware version, such as `"4.93"`.
    pub firmware: &'static str,
    /// The bus id of the region's device.
    pub bus: u64,
    /// The device id of the region's device.
    pub device: u64,
    /// The index of the region's slot in the device.
    pub slot: u64,
    /// The bus address of the region.
    pub address: u64,
    /// The size of the region, in bytes.
    pub size: u64,
}

/// Each MMIO region of devices 1 and 2 of bus 1, by firmware, device and
/// slot.
///
/// lv1's init gives the 4.93 regions of each device:
/// - device 1 [OFW-4.93 lv1 a:0x3A9A60-0x3A9A9C], which the capture
///   [`MMIO_MAP_CAPTURE`] reads in slots 0 to 2;
/// - device 2 [OFW-4.93 lv1 a:0x3A9AB4-0x3A9AF0], which the capture
///   [`MMIO_DEVICE_2_CAPTURE`] reads in slots 0 to 2.
///
/// Each capture reads the other 5 slots of its device empty.
///
/// [`MMIO_DEVICE_2_CAPTURE`]: super::MMIO_DEVICE_2_CAPTURE
/// [`MMIO_MAP_CAPTURE`]: super::MMIO_MAP_CAPTURE
pub const MMIO_REGIONS: &[MmioRegion] = &[
    MmioRegion {
        firmware: "4.93",
        bus: 1,
        device: 1,
        slot: 0,
        address: 0x2400,
        size: 0x200,
    },
    MmioRegion {
        firmware: "4.93",
        bus: 1,
        device: 1,
        slot: 1,
        address: 0x301_0000,
        size: 0x1_0000,
    },
    MmioRegion {
        firmware: "4.93",
        bus: 1,
        device: 1,
        slot: 2,
        address: 0x381_0000,
        size: 0x1_0000,
    },
    MmioRegion {
        firmware: "4.93",
        bus: 1,
        device: 2,
        slot: 0,
        address: 0x2600,
        size: 0x200,
    },
    MmioRegion {
        firmware: "4.93",
        bus: 1,
        device: 2,
        slot: 1,
        address: 0x302_0000,
        size: 0x1_0000,
    },
    MmioRegion {
        firmware: "4.93",
        bus: 1,
        device: 2,
        slot: 2,
        address: 0x382_0000,
        size: 0x1_0000,
    },
];

/// Whether a source gives every MMIO region of device `device` of bus
/// `bus` at the firmware version `firmware`.
pub fn mmio_regions_read(firmware: &str, bus: u64, device: u64) -> bool {
    MMIO_REGIONS
        .iter()
        .any(|region| region.firmware == firmware && region.bus == bus && region.device == device)
}

/// The MMIO region of device `device` of bus `bus` whose bus address is
/// `bus_address`, or `None` when the device has no such region.
///
/// The device's MMIO map function compares the address for equality
/// [OFW-4.93 lv1 a:0x228C90-0x228C98].
pub fn mmio_region(
    firmware: &str,
    bus: u64,
    device: u64,
    bus_address: u64,
) -> Option<&'static MmioRegion> {
    MMIO_REGIONS.iter().find(|region| {
        region.firmware == firmware
            && region.bus == bus
            && region.device == device
            && region.address == bus_address
    })
}

/// The real address that lv1 gives to bus address 0 of a system bus
/// device [OFW-4.93 lv1 a:0x22BE2C-0x22BE44].
pub const MMIO_REAL_BASE: u64 = 0x240_0000_0000;

/// The real address of the MMIO region at `bus_address` of device
/// `device` of bus `bus`, or `None` when [`mmio_region`] gives no region.
///
/// The device's entry at +128 passes the device's index at +128 to lv1's
/// address function [OFW-4.93 lv1 a:0x22D9C0-0x22D9D0]. That function
/// adds [`MMIO_REAL_BASE`] to the bus address. For index 6 only, it also
/// adds the doubleword at +480 of the bus object
/// [OFW-4.93 lv1 a:0x22BE1C-0x22BE68]. lv1's init creates the devices
/// of [`MMIO_REGIONS`] with these indexes, which the constructor stores
/// at +128 [OFW-4.93 lv1 a:0x3A972C] [OFW-4.93 lv1 a:0x3A9768-0x3A9780]
/// [OFW-4.93 lv1 a:0x22DD94] [OFW-4.93 lv1 a:0x22DDD8]:
/// - device 1 with index 3 [OFW-4.93 lv1 a:0x3A9A4C-0x3A9A5C];
/// - device 2 with index 4 [OFW-4.93 lv1 a:0x3A9AA0-0x3A9AB0].
pub fn mmio_real_address(firmware: &str, bus: u64, device: u64, bus_address: u64) -> Option<u64> {
    mmio_region(firmware, bus, device, bus_address).map(|region| MMIO_REAL_BASE + region.address)
}

/// The r3 of `write_htab_entry` for a valid entry whose page lies in an
/// MMIO mapping and is larger than the mapping's pages:
/// `LV1_ACCESS_VIOLATION` (-5), with r6 marked.
///
/// The MMIO map function makes the mapping's logical partition address
/// with an address function [OFW-4.93 lv1 a:0x228D68-0x228D80]. That
/// function makes a 176-byte memory region of the partition
/// [OFW-4.93 lv1 a:0x229E20-0x229E2C] [OFW-4.93 lv1 a:0x229E7C-0x229EA0]:
/// - The region keeps the logical partition address at +80
///   [OFW-4.93 lv1 a:0x2CF4C8].
/// - It keeps the call's page shift at +160 [OFW-4.93 lv1 a:0x229E84]
///   [OFW-4.93 lv1 a:0x2CF4F8].
/// - Its live byte at +164 is 0 [OFW-4.93 lv1 a:0x2CF490]
///   [OFW-4.93 lv1 a:0x2CF4FC].
/// - It keeps the real address of [`mmio_real_address`] at +168
///   [OFW-4.93 lv1 a:0x229EA8].
/// - Its class is in the TOC slot at r2 - 28088
///   [OFW-4.93 lv1 a:0x229EA4-0x229EAC] [OFW-4.93 lv1 a:0x353280].
///
/// The function then adds the region to the partition's regions
/// [OFW-4.93 lv1 a:0x229F5C-0x229F80].
///
/// `write_htab_entry` calls the method at +24 of that class
/// [OFW-4.93 lv1 a:0x35C6D8] [OFW-4.93 lv1 a:0x32D708] with the page's
/// offset in the region. The method does these steps in this order:
/// 1. It calls the method at +16 of the class
///    [OFW-4.93 lv1 a:0x22985C-0x229880] [OFW-4.93 lv1 a:0x35C6D0]
///    [OFW-4.93 lv1 a:0x32D6D8]. That method passes a region whose live
///    byte is 0 [OFW-4.93 lv1 a:0x229820-0x229828]. Any other region gets
///    the code 0x997997D2B58E3C01 [OFW-4.93 lv1 a:0x229884-0x22989C]
///    [OFW-4.93 lv1 a:0x229918].
/// 2. A page shift above the shift at +160 returns the code
///    0x997997D2B58E3C02 [OFW-4.93 lv1 a:0x2298A0-0x2298B8]
///    [OFW-4.93 lv1 a:0x2298F0-0x2298F4]. The handler gives this r3 for
///    that code [OFW-4.93 lv1 a:0x2ED5C4-0x2ED5DC]
///    [OFW-4.93 lv1 a:0x2ED60C-0x2ED618].
/// 3. The real address is the address at +168 plus the offset
///    [OFW-4.93 lv1 a:0x2298BC-0x2298C8].
///
/// The method does not read the writer's read only flag. The writer
/// stores the WIMG of an I/O range for the page; see
/// [`crate::lv1::page_table::stored_io_wimg`].
pub const WRITE_HTAB_MMIO_PAGE_SIZE_R3: u64 = 0x0000_0008_FFFF_FFFB;
