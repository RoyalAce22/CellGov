//! The device bus hypercalls: open and close, the MMIO and DMA regions,
//! the I/O segments and the IOMMU entries.
//!
//! The group answers these calls:
//! - `open_device` for devices 0, 1 and 2 of bus 1, the first devices
//!   that the 4.93 kernel opens, for the storage bus, for each device of
//!   the 4.93 storage bus table, and for each bus 4 device that lv1 finds
//!   absent;
//! - `close_device` for devices 0, 1 and 2 of bus 1, for a storage bus
//!   that the partition does not hold open, and for each bus 4 device
//!   that lv1 finds absent;
//! - `allocate_device_dma_region` and `map_device_dma_region` for device
//!   0 of bus 1;
//! - `map_device_mmio_region` for devices 0 and 1 of bus 1, and for
//!   device 2 of bus 1 that the partition does not hold open.
//!
//! [`DeviceBusState`] holds the state of the group.

mod close;
mod dma_map;
mod dma_region;
mod mmio_map;
mod open;
mod registration;

pub use dma_map::DmaMapping;
pub use dma_region::DmaRegion;
pub use mmio_map::MmioMapping;
pub use open::{OpenDevice, StorageBusOpen, StorageDeviceOpen};
pub(crate) use registration::GROUP;
pub use registration::{DeviceBusChange, DeviceBusState, StorageDevicePort};
