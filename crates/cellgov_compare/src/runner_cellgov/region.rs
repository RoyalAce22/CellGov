//! Region descriptor and the shared per-space extractor used by both
//! the scenario and boot paths.

use std::collections::BTreeMap;

use cellgov_core::AddressSpaceId;
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory, MemError, RegionAccess};

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

/// Why the extractor refused a region descriptor.
///
/// Every variant names the region so the operator can find its line
/// in the manifest that declared it.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RegionExtractError {
    /// The descriptor declares zero bytes, and an observation of nothing
    /// matches any baseline.
    #[error(
        "region {name} at 0x{addr:016x} declares zero bytes; an observation of \
         nothing compares as a match against anything, so the region cannot be \
         observed"
    )]
    Empty {
        /// Region name as declared.
        name: String,
        /// Declared start address.
        addr: u64,
    },
    /// The descriptor names an address space the run never created.
    #[error(
        "region {name} names address space {space}, but the run created \
         only spaces {present:?}; a spawned child's space is numbered from 1 \
         in spawn order, so either the title never spawned or the manifest \
         names the wrong space"
    )]
    SpaceMissing {
        /// Region name as declared.
        name: String,
        /// The space the descriptor named.
        space: u32,
        /// Every space the run created, in id order.
        present: Vec<u32>,
    },
    /// `addr + size` does not fit in the 64-bit address space.
    #[error(
        "region {name} at 0x{addr:016x} of {size} bytes runs past the end of the address space"
    )]
    Overflow {
        /// Region name as declared.
        name: String,
        /// Declared start address.
        addr: u64,
        /// Declared size in bytes.
        size: u64,
    },
    /// The space exists but does not serve the range; `source` says why.
    #[error("region {name} at 0x{addr:016x} of {size} bytes in space {space}: {source}")]
    Unreadable {
        /// Region name as declared.
        name: String,
        /// The space the descriptor named.
        space: u32,
        /// Declared start address.
        addr: u64,
        /// Declared size in bytes.
        size: u64,
        /// The memory layer's own reason.
        #[source]
        source: MemError,
    },
    /// The range lies in a `ReservedZeroReadable` region, so its bytes
    /// are provisional zeros the run never wrote.
    #[error(
        "region {name} at 0x{addr:016x} of {size} bytes in space {space} lies in \
         reserved region {region}, whose reads are provisional zeros the run never \
         wrote; an observation carries no provisional bytes, so it cannot carry this \
         region"
    )]
    Provisional {
        /// Region name as declared.
        name: String,
        /// The space the descriptor named.
        space: u32,
        /// Declared start address.
        addr: u64,
        /// Declared size in bytes.
        size: u64,
        /// Label of the reserved region the range lies in.
        region: &'static str,
    },
}

/// Read each region through its own space.
///
/// The extractor refuses a descriptor of zero bytes before it resolves
/// the space. It refuses a range in a `ReservedZeroReadable` region
/// before any read, so the snapshot's provisional-read counter does
/// not change.
///
/// # Errors
///
/// [`RegionExtractError`] for the first refused descriptor, in
/// declaration order.
pub(super) fn extract_regions(
    spaces: &SpaceSnapshots,
    regions: &[RegionDescriptor],
) -> Result<Vec<NamedMemoryRegion>, RegionExtractError> {
    regions
        .iter()
        .map(|desc| {
            if desc.size == 0 {
                return Err(RegionExtractError::Empty {
                    name: desc.name.clone(),
                    addr: desc.addr,
                });
            }
            let memory =
                spaces
                    .get(&desc.space)
                    .ok_or_else(|| RegionExtractError::SpaceMissing {
                        name: desc.name.clone(),
                        space: desc.space.raw(),
                        present: spaces.keys().map(|s| s.raw()).collect(),
                    })?;
            let range = ByteRange::new(GuestAddr::new(desc.addr), desc.size).ok_or_else(|| {
                RegionExtractError::Overflow {
                    name: desc.name.clone(),
                    addr: desc.addr,
                    size: desc.size,
                }
            })?;
            if let Some(region) = memory.containing_region(desc.addr, desc.size) {
                if region.access() == RegionAccess::ReservedZeroReadable {
                    return Err(RegionExtractError::Provisional {
                        name: desc.name.clone(),
                        space: desc.space.raw(),
                        addr: desc.addr,
                        size: desc.size,
                        region: region.label(),
                    });
                }
            }
            let data = memory
                .read_checked(range)
                .map_err(|source| RegionExtractError::Unreadable {
                    name: desc.name.clone(),
                    space: desc.space.raw(),
                    addr: desc.addr,
                    size: desc.size,
                    source,
                })?
                .to_vec();
            Ok(NamedMemoryRegion {
                name: desc.name.clone(),
                addr: desc.addr,
                data,
            })
        })
        .collect()
}

#[cfg(test)]
#[path = "tests/region_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/region_provisional_tests.rs"]
mod provisional_tests;
