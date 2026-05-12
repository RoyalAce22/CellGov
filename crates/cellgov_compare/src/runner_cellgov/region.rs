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
mod tests {
    use super::*;

    #[test]
    fn extract_returns_named_region_within_bounds() {
        let mem = vec![1u8, 2, 3, 4, 5, 6, 7, 8];
        let regions = [RegionDescriptor {
            name: "head".into(),
            addr: 0,
            size: 4,
        }];
        let extracted = extract_regions(&mem, &regions);
        assert_eq!(extracted.len(), 1);
        assert_eq!(extracted[0].name, "head");
        assert_eq!(extracted[0].addr, 0);
        assert_eq!(extracted[0].data, vec![1, 2, 3, 4]);
    }

    #[test]
    fn extract_zero_fills_when_addr_is_out_of_bounds() {
        let mem = vec![0u8; 8];
        let regions = [RegionDescriptor {
            name: "oob".into(),
            addr: 999_999,
            size: 16,
        }];
        let extracted = extract_regions(&mem, &regions);
        assert_eq!(extracted[0].data, vec![0u8; 16]);
    }

    #[test]
    fn extract_zero_fills_when_end_exceeds_memory() {
        let mem = vec![0xAA; 4];
        let regions = [RegionDescriptor {
            name: "straddle".into(),
            addr: 2,
            size: 8,
        }];
        let extracted = extract_regions(&mem, &regions);
        assert_eq!(extracted[0].data, vec![0u8; 8]);
    }

    #[test]
    fn extract_returns_empty_when_no_regions_requested() {
        let mem = vec![0u8; 32];
        let extracted = extract_regions(&mem, &[]);
        assert!(extracted.is_empty());
    }
}
