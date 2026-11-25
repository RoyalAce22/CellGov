//! The numbers of the device bus hypercalls: open and close, the MMIO and
//! DMA regions, the I/O segments and the IOMMU entries.

/// `open_device`: three inputs and no output.
/// - r3 is the id of a bus.
/// - r4 is the id of a device on that bus.
/// - r5 is the third input, which the handler gives to the device.
///
/// The lv1 handler [OFW-4.93 lv1 a:0x305A38] reads the inputs in this
/// form; see [`crate::lv1::device_bus`].
pub const OPEN_DEVICE: u64 = 170;

/// `close_device`: two inputs and no output.
/// - r3 is the id of a bus.
/// - r4 is the id of a device on that bus.
///
/// The lv1 handler [OFW-4.93 lv1 a:0x306EEC] reads the inputs in this
/// form; see [`crate::lv1::device_bus`].
pub const CLOSE_DEVICE: u64 = 171;

/// `map_device_mmio_region`: five inputs and one output.
/// - r3 is the id of a bus.
/// - r4 is the id of a device on that bus.
/// - r5 is the bus address of an MMIO region of the device.
/// - r6 is the size in bytes.
/// - r7 is the base-2 logarithm of the page size.
/// - The output r4 is the logical partition address of the mapping.
///
/// The lv1 handler [OFW-4.93 lv1 a:0x306D64] reads the inputs in this
/// form; see [`crate::lv1::device_bus`].
pub const MAP_DEVICE_MMIO_REGION: u64 = 172;

/// `unmap_device_mmio_region`: three inputs and no output.
/// - r3 is the id of a bus.
/// - r4 is the id of a device on that bus.
/// - r5 is the logical partition address of an MMIO mapping.
///
/// The lv1 handler [OFW-4.93 lv1 a:0x306C10] reads the inputs in this
/// form; see [`crate::lv1::device_bus`].
pub const UNMAP_DEVICE_MMIO_REGION: u64 = 173;

/// `allocate_device_dma_region`: five inputs and one output.
/// - r3 is the id of a bus.
/// - r4 is the id of a device on that bus.
/// - r5 is the size of the region in bytes.
/// - r6 is the base-2 logarithm of the region's page size.
/// - r7 is the region's flags.
/// - The output r4 is the bus address of the region.
///
/// The lv1 handler [OFW-4.93 lv1 a:0x306A6C] reads the inputs in this
/// form; see [`crate::lv1::device_bus`].
pub const ALLOCATE_DEVICE_DMA_REGION: u64 = 174;

/// `free_device_dma_region`: three inputs and no output.
/// - r3 is the id of a bus.
/// - r4 is the id of a device on that bus.
/// - r5 is the bus address of a DMA region of the device.
///
/// The lv1 handler [OFW-4.93 lv1 a:0x306918] reads the inputs in this
/// form; see [`crate::lv1::device_bus`].
pub const FREE_DEVICE_DMA_REGION: u64 = 175;

/// `map_device_dma_region`: six inputs and no output.
/// - r3 is the id of a bus.
/// - r4 is the id of a device on that bus.
/// - r5 is the logical partition address of the memory.
/// - r6 is the bus address in a DMA region of the device.
/// - r7 is the length in bytes.
/// - r8 is the IOPTE flags.
///
/// The lv1 handler [OFW-4.93 lv1 a:0x306770] reads the inputs in this
/// form; see [`crate::lv1::device_bus`].
pub const MAP_DEVICE_DMA_REGION: u64 = 176;

/// `unmap_device_dma_region`: four inputs and no output.
/// - r3 is the id of a bus.
/// - r4 is the id of a device on that bus.
/// - r5 is the bus address of a DMA mapping of the device.
/// - r6 is the length in bytes.
///
/// The lv1 handler [OFW-4.93 lv1 a:0x30660C] reads the inputs in this
/// form; see [`crate::lv1::device_bus`].
pub const UNMAP_DEVICE_DMA_REGION: u64 = 177;
