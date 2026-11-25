//! The MMIO mappings that lv1 made for the GameOS kernel's boot, and the
//! captures that show them.
//!
//! # The 4.93 kernel's call of `map_device_mmio_region`
//!
//! The kernel's wrapper keeps the bus id, the device id and the size from
//! r3, r4 and r6 [OFW-4.93 lv2_kernel a:0x800000000007D860-0x800000000007D868].
//! For a flags word in r7 that sets none of the bits it tests, it sends
//! the call with its own r3 to r6 and r7 = 12
//! [OFW-4.93 lv2_kernel a:0x800000000007D894-0x800000000007D8CC]. When r3
//! is 0, it keeps r4 as the logical partition address
//! [OFW-4.93 lv2_kernel a:0x800000000007D8D0-0x800000000007D8DC]. The
//! kernel's boot sends the call for the regions in slots 1 and 2 of
//! device 1, and then for the same slots of device 2. The captures
//! [`MMIO_MAP_CAPTURE`], [`MMIO_LISTS_CAPTURE`] and
//! [`MMIO_DEVICE_2_CAPTURE`] give lv1's record of each call; see
//! [`GAME_OS_MMIO_MAPPINGS`].

/// The MMIO mapping that the console's lv1 made for one call of the
/// GameOS kernel's boot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GameOsMmioMapping {
    /// The firmware version, such as `"4.93"`.
    pub firmware: &'static str,
    /// The bus id of the call.
    pub bus: u64,
    /// The device id of the call.
    pub device: u64,
    /// The bus address of the call.
    pub bus_address: u64,
    /// The size of the call, in bytes.
    pub size: u64,
    /// The page shift of the call.
    pub page_shift: u64,
    /// The logical partition address that lv1 gave the call.
    pub lpar_address: u64,
}

/// Each MMIO mapping of a kernel's boot calls, by firmware, in the order
/// of the calls.
///
/// Each 4.93 row is the one game OS mapping of its region of device 1
/// or 2:
/// - the capture [`MMIO_MAP_CAPTURE`] finds the first row in the list of
///   the region in slot 1 of device 1;
/// - the capture [`MMIO_LISTS_CAPTURE`] finds the second row in the list
///   of the region in slot 2 of device 1;
/// - the capture [`MMIO_DEVICE_2_CAPTURE`] finds the third and fourth
///   rows in the lists of the regions in slots 1 and 2 of device 2.
///
/// Each mapping has partition 2 at +8, the address at +48 and +80 and
/// the length at +56. The page shift is the one that the kernel's
/// wrapper sends [OFW-4.93 lv2_kernel a:0x800000000007D8B0]. The
/// addresses are those of the region counter values 7 to 10 in the
/// encoding of [`crate::lv1::memory`].
pub const GAME_OS_MMIO_MAPPINGS: &[GameOsMmioMapping] = &[
    GameOsMmioMapping {
        firmware: "4.93",
        bus: 1,
        device: 1,
        bus_address: 0x301_0000,
        size: 0x1_0000,
        page_shift: 12,
        lpar_address: 0x4000_0007_0000,
    },
    GameOsMmioMapping {
        firmware: "4.93",
        bus: 1,
        device: 1,
        bus_address: 0x381_0000,
        size: 0x1_0000,
        page_shift: 12,
        lpar_address: 0x4000_0008_0000,
    },
    GameOsMmioMapping {
        firmware: "4.93",
        bus: 1,
        device: 2,
        bus_address: 0x302_0000,
        size: 0x1_0000,
        page_shift: 12,
        lpar_address: 0x4000_0009_0000,
    },
    GameOsMmioMapping {
        firmware: "4.93",
        bus: 1,
        device: 2,
        bus_address: 0x382_0000,
        size: 0x1_0000,
        page_shift: 12,
        lpar_address: 0x4000_000A_0000,
    },
];

/// The console capture of the MMIO regions of device 1 of bus 1 and the
/// mappings that lv1 holds in its second region.
///
/// The path is relative to the workspace root. Its README gives the
/// method and the analysis.
pub const MMIO_MAP_CAPTURE: &str = "tests/micro/lv1_device_mmio_map";

/// The console capture of the mappings that lv1 holds in the first and
/// third MMIO regions of device 1 of bus 1.
///
/// The path is relative to the workspace root. Its README gives the
/// method and the analysis.
pub const MMIO_LISTS_CAPTURE: &str = "tests/micro/lv1_device_mmio_lists";

/// The console capture of the MMIO and DMA regions of device 2 of bus 1
/// and the mappings that lv1 holds in its MMIO regions.
///
/// The path is relative to the workspace root. Its README gives the
/// method and the analysis.
pub const MMIO_DEVICE_2_CAPTURE: &str = "tests/micro/lv1_device_2_regions";
