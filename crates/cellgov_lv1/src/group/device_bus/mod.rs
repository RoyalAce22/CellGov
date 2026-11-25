//! The device bus hypercalls: open and close, the MMIO and DMA regions,
//! the I/O segments and the IOMMU entries.
//!
//! The group answers `open_device` for device 0 of bus 1, the device
//! that the 4.93 kernel opens. The state holds each device that the
//! partition opened.

mod open;
mod registration;

pub use open::OpenDevice;
pub(crate) use registration::GROUP;
pub use registration::{DeviceBusChange, DeviceBusState};
