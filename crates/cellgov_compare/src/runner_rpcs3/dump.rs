//! Memory-dump region extractor.

use std::path::Path;

use crate::observation::NamedMemoryRegion;

use super::config::DumpRegion;
use super::error::Rpcs3Error;

/// Read a memory dump file and extract the declared regions.
pub fn parse_dump(
    dump_path: &Path,
    regions: &[DumpRegion],
) -> Result<Vec<NamedMemoryRegion>, Rpcs3Error> {
    let data = std::fs::read(dump_path).map_err(Rpcs3Error::DumpRead)?;
    let data_len = data.len() as u64;
    let mut result = Vec::with_capacity(regions.len());

    for region in regions {
        let end = region.offset.checked_add(region.size).ok_or_else(|| {
            Rpcs3Error::DumpOffsetOverflow {
                region_name: region.name.clone(),
                offset: region.offset,
                size: region.size,
            }
        })?;
        if end > data_len {
            return Err(Rpcs3Error::DumpTooSmall {
                region_name: region.name.clone(),
                guest_addr: region.guest_addr,
                expected: end,
                actual: data_len,
            });
        }
        let start = region.offset as usize;
        let end_usz = end as usize;
        result.push(NamedMemoryRegion {
            name: region.name.clone(),
            addr: region.guest_addr,
            data: data[start..end_usz].to_vec(),
        });
    }

    Ok(result)
}

#[cfg(test)]
#[path = "tests/dump_tests.rs"]
mod tests;
