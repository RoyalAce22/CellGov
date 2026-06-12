//! Region descriptor and the shared `[u8] + &[RegionDescriptor]`
//! extractor used by both the scenario and boot paths.

use crate::observation::NamedMemoryRegion;

/// Memory region to extract from final committed memory (guest address space).
#[derive(Debug, Clone)]
pub struct RegionDescriptor {
    /// Region name for the observation.
    pub name: String,
    /// Guest address of the region start.
    pub addr: u64,
    /// Size in bytes.
    pub size: u64,
}

/// Slice `regions` out of `memory`; out-of-bounds regions are filled
/// with zeros so the comparison layer reports the mismatch as a normal
/// memory divergence rather than a runner-side panic.
pub(super) fn extract_regions(
    memory: &[u8],
    regions: &[RegionDescriptor],
) -> Vec<NamedMemoryRegion> {
    regions
        .iter()
        .map(|desc| {
            let start = desc.addr as usize;
            let end = start.saturating_add(desc.size as usize);
            let data = if start <= memory.len() && end <= memory.len() {
                memory[start..end].to_vec()
            } else {
                vec![0u8; desc.size as usize]
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
