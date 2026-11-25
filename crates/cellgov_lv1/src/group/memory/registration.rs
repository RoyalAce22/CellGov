//! The memory group's registration, its state and the changes an answer
//! carries to it.

use cellgov_mem::scheme::{HashedSurface, Stamp};

use super::address_space::{self, AddressSpaceChange, AddressSpaceState};
use super::page_table;
use super::region::{self, MemoryRegion, MemoryState};
use crate::group::state::StateChange;
use crate::group::table::Group;

/// The memory group's registration.
pub(crate) const GROUP: Group = Group {
    name: "memory",
    hypercalls: &[
        region::HYPERCALLS,
        address_space::HYPERCALLS,
        page_table::HYPERCALLS,
    ],
    lanes: &[77..=80, 128..=143],
    hashed_surfaces: &[
        HashedSurface {
            name: "the memory regions",
            source: Some(REGION_SOURCE),
            stamp: Stamp::Version(37),
        },
        HashedSurface {
            name: "the region counter",
            source: Some(REGION_COUNTER_SOURCE),
            stamp: Stamp::Version(22),
        },
        HashedSurface {
            name: "the virtual address spaces",
            source: Some(ADDRESS_SPACE_SOURCE),
            stamp: Stamp::Version(23),
        },
        HashedSurface {
            name: "the selected virtual address space",
            source: Some(SELECTED_ADDRESS_SPACE_SOURCE),
            stamp: Stamp::Version(23),
        },
        HashedSurface {
            name: "the real address of each memory region's storage",
            source: Some(REGION_STORAGE_SOURCE),
            stamp: Stamp::Version(37),
        },
        HashedSurface {
            name: "the real address of each address space's page table",
            source: Some(PAGE_TABLE_STORAGE_SOURCE),
            stamp: Stamp::Version(37),
        },
        HashedSurface {
            name: "the end of the storage that the memory regions got",
            source: Some(STORAGE_END_SOURCE),
            stamp: Stamp::Version(37),
        },
    ],
};

/// The lane source of the memory regions the hypervisor created, one
/// object per region, keyed by its counter value.
pub(crate) const REGION_SOURCE: u8 = 77;

/// The lane source of the partition's region counter.
pub(crate) const REGION_COUNTER_SOURCE: u8 = 78;

/// The lane source of the virtual address spaces the hypervisor
/// constructed, one object per address space in the order of
/// construction.
pub(crate) const ADDRESS_SPACE_SOURCE: u8 = 79;

/// The lane source of the vas id that the partition's PPE selected.
pub(crate) const SELECTED_ADDRESS_SPACE_SOURCE: u8 = 80;

/// The lane source of the real addresses of the regions' storage, one
/// object per region that has storage, keyed by its counter value.
pub(crate) const REGION_STORAGE_SOURCE: u8 = 128;

/// The lane source of the real address past the last range of storage
/// that a memory region got.
pub(crate) const STORAGE_END_SOURCE: u8 = 130;

/// The lane source of the real addresses of the page tables, one object
/// per address space whose table has storage, keyed by its construction
/// index.
pub(crate) const PAGE_TABLE_STORAGE_SOURCE: u8 = 129;

/// The memory group's state: the partition's memory regions and its
/// virtual address spaces.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MemoryGroupState {
    regions: MemoryState,
    spaces: AddressSpaceState,
}

impl MemoryGroupState {
    /// The partition's memory regions, their storage and its region counter.
    pub fn regions(&self) -> &MemoryState {
        &self.regions
    }

    /// The partition's virtual address spaces and the selected one.
    pub fn spaces(&self) -> &AddressSpaceState {
        &self.spaces
    }

    /// Applies `change`. A page table mapping also moves the region
    /// counter past the value it took.
    pub(crate) fn apply(&mut self, change: MemoryChange) {
        match change {
            MemoryChange::Region(region) => self.regions.create(region),
            MemoryChange::CounterTaken(counter) => self.regions.take_counter(counter),
            MemoryChange::AddressSpace(change) => {
                if let AddressSpaceChange::Mapped { counter, .. } = change {
                    self.regions.take_counter(counter);
                }
                self.spaces.apply(change);
            }
        }
    }

    /// The sync-state terms of the regions and the address spaces.
    pub fn sync_term(&self) -> u128 {
        self.regions
            .sync_term()
            .wrapping_add(self.spaces.sync_term())
    }
}

/// A change to the memory group's state that an answer carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryChange {
    /// The call created this memory region.
    Region(MemoryRegion),
    /// A call of another group created regions, and this is the last
    /// value of the region counter that they took.
    CounterTaken(u64),
    /// The call changed the virtual address spaces.
    AddressSpace(AddressSpaceChange),
}

impl From<MemoryChange> for StateChange {
    fn from(change: MemoryChange) -> Self {
        StateChange::Memory(Box::new(change))
    }
}

impl From<MemoryRegion> for StateChange {
    fn from(region: MemoryRegion) -> Self {
        MemoryChange::Region(region).into()
    }
}

impl From<AddressSpaceChange> for StateChange {
    fn from(change: AddressSpaceChange) -> Self {
        MemoryChange::AddressSpace(change).into()
    }
}
