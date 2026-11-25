//! The device bus group's registration, its state and the changes an
//! answer carries to it.

use std::collections::{BTreeMap, BTreeSet};

use cellgov_mem::lanes::{self, LaneValue, ObjectLanes};
use cellgov_mem::scheme::{HashedSurface, Stamp};
use cellgov_ps3_abi::lv1::device_bus::{opens_storage_bus, STORAGE_BUS};

use super::close;
use super::dma_map::{self, DmaMapping};
use super::dma_region::{self, DmaRegion};
use super::mmio_map::{self, MmioMapping};
use super::open::{self, OpenDevice, StorageBusOpen, StorageDeviceOpen};
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
        mmio_map::HYPERCALLS,
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
            stamp: Stamp::Version(33),
        },
        HashedSurface {
            name: "the storage devices that the partition opened",
            source: Some(STORAGE_DEVICE_OPEN_SOURCE),
            stamp: Stamp::Version(34),
        },
        HashedSurface {
            name: "the event receive ports that lv1 connected to the storage devices",
            source: Some(STORAGE_DEVICE_PORT_SOURCE),
            stamp: Stamp::Version(34),
        },
        HashedSurface {
            name: "the MMIO mappings that lv1 made for the partition",
            source: Some(MMIO_MAPPING_SOURCE),
            stamp: Stamp::Version(34),
        },
        HashedSurface {
            name: "the page shift of each MMIO mapping",
            source: Some(MMIO_PAGE_SHIFT_SOURCE),
            stamp: Stamp::Version(35),
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
/// storage bus: object 0, whose field 1 holds the port id.
pub(crate) const STORAGE_BUS_PORT_SOURCE: u8 = 180;

/// The lane source of the storage device opens: one object for each open
/// storage device, keyed by its index in device order.
pub(crate) const STORAGE_DEVICE_OPEN_SOURCE: u8 = 181;

/// The lane source of the event receive ports that lv1 connected to the
/// storage devices: one object for each connected device, keyed by its
/// index in device order.
pub(crate) const STORAGE_DEVICE_PORT_SOURCE: u8 = 182;

/// The lane source of the MMIO mappings: one object for each mapping,
/// keyed by its index in the order of [`DeviceBusState::mmio_mappings`].
pub(crate) const MMIO_MAPPING_SOURCE: u8 = 183;

/// The lane source of the page shifts of the MMIO mappings: one object
/// for each mapping, with the key of [`MMIO_MAPPING_SOURCE`].
pub(crate) const MMIO_PAGE_SHIFT_SOURCE: u8 = 184;

/// The event receive port that lv1 connected to the partition's slot of
/// one storage device.
///
/// lv1 keeps the port at +16 of the slot's handle in the device object
/// [OFW-4.93 lv1 a:0x25554C-0x255570] [OFW-4.93 lv1 a:0x2BEDD4].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StorageDevicePort {
    /// The device id on the storage bus, from r4.
    pub device: u64,
    /// The port id, from r5.
    pub port: u64,
}

/// Fields 1 and 2: the device id and the port id.
impl LaneValue for StorageDevicePort {
    fn lanes(&self, lanes: &mut ObjectLanes) {
        lanes.lane(1, 0, self.device);
        lanes.lane(2, 0, self.port);
    }
}

/// The device bus group's state.
///
/// The state holds these values:
/// - each device that the partition opened;
/// - each DMA region that lv1 gave the partition;
/// - each mapping that lv1 made in those regions;
/// - the storage bus open that the partition holds;
/// - the event receive port that lv1 connected to the storage bus;
/// - each storage device open that the partition holds;
/// - the event receive port that lv1 connected to each storage device;
/// - each mapping that lv1 made in an MMIO region for the partition.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DeviceBusState {
    open: BTreeSet<OpenDevice>,
    regions: BTreeSet<DmaRegion>,
    mappings: BTreeSet<DmaMapping>,
    storage_bus: Option<StorageBusOpen>,
    storage_bus_port: Option<u64>,
    storage_devices: BTreeMap<u64, StorageDeviceOpen>,
    storage_device_ports: BTreeMap<u64, StorageDevicePort>,
    mmio_mappings: BTreeSet<MmioMapping>,
}

impl DeviceBusState {
    /// Each MMIO mapping, in bus, device, bus address, length and logical
    /// partition address order.
    pub fn mmio_mappings(&self) -> impl Iterator<Item = MmioMapping> + '_ {
        self.mmio_mappings.iter().copied()
    }

    /// The partition's first MMIO mapping of the region at `bus_address`
    /// of device `device` of bus `bus`, if any.
    ///
    /// A region can hold more than one mapping. The first mapping has the
    /// least length, then the least logical partition address.
    pub fn mmio_mapping(&self, bus: u64, device: u64, bus_address: u64) -> Option<MmioMapping> {
        self.mmio_mappings().find(|mapping| {
            mapping.bus == bus && mapping.device == device && mapping.bus_address == bus_address
        })
    }

    /// The port that lv1 connected to the partition's slot of storage
    /// device `device`, if any.
    pub fn storage_device_port(&self, device: u64) -> Option<StorageDevicePort> {
        self.storage_device_ports.get(&device).copied()
    }

    /// Each port that lv1 connected to a storage device, in device order.
    pub fn storage_device_ports(&self) -> impl Iterator<Item = StorageDevicePort> + '_ {
        self.storage_device_ports.values().copied()
    }

    /// The partition's open of storage device `device`, if any.
    pub fn storage_device_open(&self, device: u64) -> Option<StorageDeviceOpen> {
        self.storage_devices.get(&device).copied()
    }

    /// Each storage device open that the partition holds, in device order.
    pub fn storage_device_opens(&self) -> impl Iterator<Item = StorageDeviceOpen> + '_ {
        self.storage_devices.values().copied()
    }

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
            DeviceBusChange::StorageDeviceOpened(open) => {
                self.storage_devices.insert(open.device, open);
            }
            DeviceBusChange::StorageDevicePortConnected(port) => {
                self.storage_device_ports.insert(port.device, port);
            }
            DeviceBusChange::MmioRegionMapped(mapping) => {
                self.mmio_mappings.insert(mapping);
            }
            DeviceBusChange::Closed(device) => {
                self.open.remove(&device);
                self.mmio_mappings
                    .retain(|mapping| mapping.bus != device.bus || mapping.device != device.device);
                self.mappings
                    .retain(|mapping| mapping.bus != device.bus || mapping.device != device.device);
                self.regions
                    .retain(|region| region.bus != device.bus || region.device != device.device);
                self.release_storage_slot(device);
            }
        }
    }

    /// Removes the partition's open of the storage bus or storage device
    /// `device`, and the port that lv1 connected to that slot.
    ///
    /// lv1's storage connect takes only a used slot that holds the
    /// caller's partition. It gives -9 only while the slot's handle holds
    /// a port with a reference [OFW-4.93 lv1 a:0x2554F0-0x25551C]
    /// [OFW-4.93 lv1 a:0x255520-0x255548]. A kept port after a close would
    /// thus give a busy answer that lv1 cannot give.
    fn release_storage_slot(&mut self, device: OpenDevice) {
        if opens_storage_bus(device.bus, device.device) {
            self.storage_bus = None;
            self.storage_bus_port = None;
            return;
        }
        if device.bus != STORAGE_BUS {
            return;
        }
        self.storage_devices.remove(&device.device);
        self.storage_device_ports.remove(&device.device);
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
        let storage_bus_port = self.storage_bus_port.map_or(storage_bus, |id| {
            storage_bus.wrapping_add(lanes::value_term(STORAGE_BUS_PORT_SOURCE, 0, &id))
        });
        let storage_devices = (0u64..).zip(self.storage_device_opens()).fold(
            storage_bus_port,
            |sum, (index, open)| {
                sum.wrapping_add(lanes::value_term(STORAGE_DEVICE_OPEN_SOURCE, index, &open))
            },
        );
        let storage_device_ports = (0u64..).zip(self.storage_device_ports()).fold(
            storage_devices,
            |sum, (index, port)| {
                sum.wrapping_add(lanes::value_term(STORAGE_DEVICE_PORT_SOURCE, index, &port))
            },
        );
        (0u64..)
            .zip(self.mmio_mappings())
            .fold(storage_device_ports, |sum, (index, mapping)| {
                sum.wrapping_add(lanes::value_term(MMIO_MAPPING_SOURCE, index, &mapping))
                    .wrapping_add(lanes::value_term(
                        MMIO_PAGE_SHIFT_SOURCE,
                        index,
                        &mapping.page_shift,
                    ))
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
    /// The call opened this storage device.
    StorageDeviceOpened(StorageDeviceOpen),
    /// The call connected this event receive port to the partition's slot
    /// of a storage device.
    StorageDevicePortConnected(StorageDevicePort),
    /// lv1 made this MMIO mapping for the partition.
    MmioRegionMapped(MmioMapping),
    /// The call closed this device. The change removes these values of
    /// the device:
    /// - each DMA mapping, which lv1 removed;
    /// - each DMA region, which lv1 freed;
    /// - each MMIO mapping, which lv1 removed;
    /// - for a device of the storage bus, or the storage bus itself, the
    ///   partition's open and the port of its slot.
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

#[cfg(test)]
#[path = "tests/storage_device_open_hash_tests.rs"]
mod storage_device_open_hash_tests;

#[cfg(test)]
#[path = "tests/storage_device_port_hash_tests.rs"]
mod storage_device_port_hash_tests;

#[cfg(test)]
#[path = "tests/mmio_map_hash_tests.rs"]
mod mmio_map_hash_tests;

#[cfg(test)]
#[path = "tests/mmio_page_shift_hash_tests.rs"]
mod mmio_page_shift_hash_tests;

#[cfg(test)]
#[path = "tests/storage_close_state_tests.rs"]
mod storage_close_state_tests;
