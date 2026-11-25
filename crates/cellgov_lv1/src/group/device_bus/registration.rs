//! The device bus group's registration, its state and the changes an
//! answer carries to it.

use std::collections::BTreeSet;

use cellgov_mem::lanes;
use cellgov_mem::scheme::{HashedSurface, Stamp};

use super::dma_region::{self, DmaRegion};
use super::open::{self, OpenDevice};
use crate::group::state::StateChange;
use crate::group::table::Group;

/// The device bus group's registration.
pub(crate) const GROUP: Group = Group {
    name: "device_bus",
    hypercalls: &[open::HYPERCALLS, dma_region::HYPERCALLS],
    lanes: &[176..=191],
    hashed_surfaces: &[
        HashedSurface {
            name: "the devices that the partition opened",
            source: Some(OPEN_DEVICE_SOURCE),
            stamp: Stamp::Version(31),
        },
        HashedSurface {
            name: "the DMA regions that the partition allocated",
            source: Some(DMA_REGION_SOURCE),
            stamp: Stamp::Pending,
        },
    ],
};

/// The lane source of the open devices: one object for each open device,
/// keyed by its index in bus and device order.
pub(crate) const OPEN_DEVICE_SOURCE: u8 = 176;

/// The lane source of the DMA regions: one object for each region, keyed
/// by its index in bus, device and address order.
pub(crate) const DMA_REGION_SOURCE: u8 = 177;

/// The device bus group's state.
///
/// The state holds these values:
/// - each device that the partition opened;
/// - each DMA region that lv1 gave the partition.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DeviceBusState {
    open: BTreeSet<OpenDevice>,
    regions: BTreeSet<DmaRegion>,
}

impl DeviceBusState {
    /// Each open device, in bus and device order.
    pub fn open_devices(&self) -> impl Iterator<Item = OpenDevice> + '_ {
        self.open.iter().copied()
    }

    /// Whether the partition opened `device`.
    pub fn is_open(&self, device: OpenDevice) -> bool {
        self.open.contains(&device)
    }

    /// Each DMA region, in bus, device and address order.
    pub fn dma_regions(&self) -> impl Iterator<Item = DmaRegion> + '_ {
        self.regions.iter().copied()
    }

    /// Each DMA region of `device`, in address order.
    pub fn regions_of(&self, device: OpenDevice) -> impl Iterator<Item = DmaRegion> + '_ {
        self.dma_regions()
            .filter(move |region| region.bus == device.bus && region.device == device.device)
    }

    /// Applies `change` to the state.
    pub(crate) fn apply(&mut self, change: DeviceBusChange) {
        match change {
            DeviceBusChange::Opened(device) => {
                self.open.insert(device);
            }
            DeviceBusChange::DmaRegionAllocated(region) => {
                self.regions.insert(region);
            }
        }
    }

    /// The sync-state terms of the open devices and the DMA regions. A
    /// state with no open device and no region adds no term.
    pub fn sync_term(&self) -> u128 {
        let open = (0u64..)
            .zip(self.open_devices())
            .fold(0u128, |sum, (index, device)| {
                sum.wrapping_add(lanes::value_term(OPEN_DEVICE_SOURCE, index, &device))
            });
        (0u64..)
            .zip(self.dma_regions())
            .fold(open, |sum, (index, region)| {
                sum.wrapping_add(lanes::value_term(DMA_REGION_SOURCE, index, &region))
            })
    }
}

/// A change to the device bus group's state that an answer carries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeviceBusChange {
    /// The call opened this device.
    Opened(OpenDevice),
    /// lv1 gave the partition this DMA region.
    DmaRegionAllocated(DmaRegion),
}

impl From<DeviceBusChange> for StateChange {
    fn from(change: DeviceBusChange) -> Self {
        StateChange::DeviceBus(Box::new(change))
    }
}

#[cfg(test)]
#[path = "tests/open_device_hash_tests.rs"]
mod hash_tests;

#[cfg(test)]
#[path = "tests/dma_region_hash_tests.rs"]
mod dma_region_hash_tests;
