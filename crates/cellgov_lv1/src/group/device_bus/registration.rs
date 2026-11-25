//! The device bus group's registration, its state and the changes an
//! answer carries to it.

use std::collections::BTreeSet;

use cellgov_mem::lanes;
use cellgov_mem::scheme::{HashedSurface, Stamp};

use super::close;
use super::dma_map::{self, DmaMapping};
use super::dma_region::{self, DmaRegion};
use super::open::{self, OpenDevice, StorageBusOpen};
use crate::group::state::StateChange;
use crate::group::table::Group;

/// The device bus group's registration.
pub(crate) const GROUP: Group = Group {
    name: "device_bus",
    hypercalls: &[
        open::HYPERCALLS,
        close::HYPERCALLS,
        dma_region::HYPERCALLS,
        dma_map::HYPERCALLS,
    ],
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
            stamp: Stamp::Version(32),
        },
        HashedSurface {
            name: "the DMA mappings that lv1 made for the partition",
            source: Some(DMA_MAPPING_SOURCE),
            stamp: Stamp::Version(32),
        },
        HashedSurface {
            name: "the storage bus open that the partition holds",
            source: Some(STORAGE_BUS_OPEN_SOURCE),
            stamp: Stamp::Version(32),
        },
        HashedSurface {
            name: "the event receive port that lv1 connected to the storage bus",
            source: Some(STORAGE_BUS_PORT_SOURCE),
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

/// The lane source of the DMA mappings: one object for each mapping,
/// keyed by its index in bus, device and bus address order.
pub(crate) const DMA_MAPPING_SOURCE: u8 = 178;

/// The lane source of the storage bus open: object 0 while the partition
/// holds the storage bus open.
pub(crate) const STORAGE_BUS_OPEN_SOURCE: u8 = 179;

/// The lane source of the event receive port that lv1 connected to the
/// storage bus: one object, whose field 1 holds the port id.
pub(crate) const STORAGE_BUS_PORT_SOURCE: u8 = 180;

/// The device bus group's state.
///
/// The state holds these values:
/// - each device that the partition opened;
/// - each DMA region that lv1 gave the partition;
/// - each mapping that lv1 made in those regions;
/// - the storage bus open that the partition holds;
/// - the event receive port that lv1 connected to the storage bus.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DeviceBusState {
    open: BTreeSet<OpenDevice>,
    regions: BTreeSet<DmaRegion>,
    mappings: BTreeSet<DmaMapping>,
    storage_bus: Option<StorageBusOpen>,
    storage_bus_port: Option<u64>,
}

impl DeviceBusState {
    /// The storage bus open that the partition holds, if any.
    pub fn storage_bus_open(&self) -> Option<StorageBusOpen> {
        self.storage_bus
    }

    /// The id of the event receive port that lv1 connected to the
    /// partition's slot of the storage bus, or `None` when lv1 connected
    /// no port.
    pub fn storage_bus_port(&self) -> Option<u64> {
        self.storage_bus_port
    }

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

    /// Each DMA mapping, in bus, device and bus address order.
    pub fn dma_mappings(&self) -> impl Iterator<Item = DmaMapping> + '_ {
        self.mappings.iter().copied()
    }

    /// Each DMA mapping of `device`, in bus address order.
    pub fn mappings_of(&self, device: OpenDevice) -> impl Iterator<Item = DmaMapping> + '_ {
        self.dma_mappings()
            .filter(move |mapping| mapping.bus == device.bus && mapping.device == device.device)
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
            DeviceBusChange::DmaRegionMapped(mapping) => {
                self.mappings.insert(mapping);
            }
            DeviceBusChange::StorageBusOpened(open) => {
                self.storage_bus = Some(open);
            }
            DeviceBusChange::StorageBusPortConnected(id) => {
                self.storage_bus_port = Some(id);
            }
            DeviceBusChange::Closed(device) => {
                self.open.remove(&device);
                self.mappings
                    .retain(|mapping| mapping.bus != device.bus || mapping.device != device.device);
                self.regions
                    .retain(|region| region.bus != device.bus || region.device != device.device);
            }
        }
    }

    /// The sum of the sync-state terms of the group's hashed surfaces.
    ///
    /// A state that holds no value adds no term.
    pub fn sync_term(&self) -> u128 {
        let open = (0u64..)
            .zip(self.open_devices())
            .fold(0u128, |sum, (index, device)| {
                sum.wrapping_add(lanes::value_term(OPEN_DEVICE_SOURCE, index, &device))
            });
        let regions = (0u64..)
            .zip(self.dma_regions())
            .fold(open, |sum, (index, region)| {
                sum.wrapping_add(lanes::value_term(DMA_REGION_SOURCE, index, &region))
            });
        let mappings = (0u64..)
            .zip(self.dma_mappings())
            .fold(regions, |sum, (index, mapping)| {
                sum.wrapping_add(lanes::value_term(DMA_MAPPING_SOURCE, index, &mapping))
            });
        let storage_bus = self.storage_bus.map_or(mappings, |open| {
            mappings.wrapping_add(lanes::value_term(STORAGE_BUS_OPEN_SOURCE, 0, &open))
        });
        self.storage_bus_port.map_or(storage_bus, |id| {
            storage_bus.wrapping_add(lanes::value_term(STORAGE_BUS_PORT_SOURCE, 0, &id))
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
    /// lv1 made this DMA mapping for the partition.
    DmaRegionMapped(DmaMapping),
    /// The call opened the storage bus.
    StorageBusOpened(StorageBusOpen),
    /// The call connected the event receive port with this id to the
    /// partition's slot of the storage bus.
    StorageBusPortConnected(u64),
    /// The call closed this device. lv1 removed each DMA mapping of the
    /// device and then freed each DMA region of the device.
    Closed(OpenDevice),
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

#[cfg(test)]
#[path = "tests/dma_map_hash_tests.rs"]
mod dma_map_hash_tests;

#[cfg(test)]
#[path = "tests/storage_bus_open_hash_tests.rs"]
mod storage_bus_open_hash_tests;

#[cfg(test)]
#[path = "tests/storage_bus_port_hash_tests.rs"]
mod storage_bus_port_hash_tests;
