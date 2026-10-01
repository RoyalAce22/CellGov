//! Volatile-byte masking: the bytes a hardware run legitimately varies,
//! zeroed on each side before two observations are compared.

use crate::manifest::VolatileRange;

use super::Observation;

/// Zero every byte `ranges` declare in `observation`'s regions.
///
/// Each range names a region and a byte span from the region start. A
/// range whose region the observation lacks, or the part of a range
/// past the end of a region's data, blanks nothing: the comparison that
/// follows already reports a missing region or a length difference,
/// and blanking never hides one.
pub fn blank_volatile(observation: &mut Observation, ranges: &[VolatileRange]) {
    for range in ranges {
        for region in observation
            .memory_regions
            .iter_mut()
            .filter(|r| r.name == range.region)
        {
            let len = region.data.len() as u64;
            let start = range.offset.min(len);
            let end = range.offset.saturating_add(range.size).min(len);
            // start <= end <= len, and len came from a usize.
            region.data[start as usize..end as usize].fill(0);
        }
    }
}

#[cfg(test)]
#[path = "tests/mask_tests.rs"]
mod tests;
