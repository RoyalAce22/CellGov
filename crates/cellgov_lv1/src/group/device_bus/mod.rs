//! The device bus hypercalls: open and close, the MMIO and DMA regions,
//! the I/O segments and the IOMMU entries.
//!
//! The group answers these calls:
//! - `open_device` for devices 0, 1 and 2 of bus 1, the first devices
//!   that the 4.93 kernel opens;
//! - `allocate_device_dma_region` and `map_device_dma_region` for device
//!   0 of bus 1.
//!
//! The state holds each device that the partition opened, each DMA region
//! that lv1 gave it, and each mapping that lv1 made in those regions.

mod dma_map;
mod dma_region;
mod open;
mod registration;

pub use dma_map::DmaMapping;
pub use dma_region::DmaRegion;
pub use open::OpenDevice;
pub(crate) use registration::GROUP;
pub use registration::{DeviceBusChange, DeviceBusState};
