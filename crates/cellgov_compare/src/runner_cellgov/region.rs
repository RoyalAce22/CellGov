//! Region descriptor and the shared per-space extractor used by both
//! the scenario and boot paths.

use std::collections::BTreeMap;

use cellgov_core::AddressSpaceId;
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};

use crate::observation::NamedMemoryRegion;

/// Every address space of a finished run, keyed by id; space 0 is the
/// boot process's.
pub type SpaceSnapshots = BTreeMap<AddressSpaceId, GuestMemory>;

/// Memory region to extract from a run's final committed memory.
#[derive(Debug, Clone)]
pub struct RegionDescriptor {
    /// Region name for the observation.
    pub name: String,
    /// Address space the region lives in.
    pub space: AddressSpaceId,
    /// Guest address of the region start.
    pub addr: u64,
    /// Size in bytes.
    pub size: u64,
}

/// Read each region through its own space; a region naming an unknown
/// space, or one `GuestMemory::read` cannot resolve, is zero-filled so
/// the comparison layer reports it as a memory divergence. A
/// `ReservedZeroReadable` target reads as zeros and bumps the
/// snapshot's provisional-read counter.
pub(super) fn extract_regions(
    spaces: &SpaceSnapshots,
    regions: &[RegionDescriptor],
) -> Vec<NamedMemoryRegion> {
    regions
        .iter()
        .map(|desc| {
            let bytes = spaces.get(&desc.space).and_then(|memory| {
                ByteRange::new(GuestAddr::new(desc.addr), desc.size)
                    .and_then(|range| memory.read(range))
            });
            let data = match bytes {
                Some(b) => b.to_vec(),
                None => vec![0u8; desc.size as usize],
            };
            NamedMemoryRegion {
                name: desc.name.clone(),
                addr: desc.addr,
                data,
            }
        })
        .collect()
}

#[cfg(test)]
#[path = "tests/region_tests.rs"]
mod tests;
