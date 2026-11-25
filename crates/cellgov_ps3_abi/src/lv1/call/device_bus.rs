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
