//! TTY-log frame format and extractor.
//!
//! # TTY frame format
//!
//! The test writes `CGOV` (4 bytes) + big-endian u32 payload length +
//! raw payload bytes via `sys_tty_write`. The adapter locates the magic
//! tag in the log and slices regions from the payload in declaration
//! order.

use std::path::Path;

use crate::observation::NamedMemoryRegion;

use super::config::TtyRegion;
use super::error::Rpcs3Error;

/// Magic tag that precedes the big-endian u32 length and payload bytes.
pub const TTY_MAGIC: &[u8; 4] = b"CGOV";

/// TTY frame header: 4-byte magic + 4-byte length.
const TTY_HEADER_SIZE: usize = 8;

/// Scan a TTY log for the `CGOV` frame and slice the declared regions
/// from its payload in declaration order.
pub fn parse_tty_log(
    tty_path: &Path,
    regions: &[TtyRegion],
) -> Result<Vec<NamedMemoryRegion>, Rpcs3Error> {
    let data = std::fs::read(tty_path).map_err(Rpcs3Error::TtyRead)?;

    let magic_pos = data
        .windows(TTY_MAGIC.len())
        .position(|w| w == TTY_MAGIC.as_slice())
        .ok_or(Rpcs3Error::TtyMagicNotFound)?;

    // `magic_pos < data.len()` by find-position contract, so the
    // header_end add is bounded by `data.len() + TTY_HEADER_SIZE`,
    // well below `usize::MAX` for any real TTY log.
    let header_end = magic_pos + TTY_HEADER_SIZE;
    if header_end > data.len() {
        return Err(Rpcs3Error::TtyPayloadTooSmall {
            expected: TTY_HEADER_SIZE as u64,
            actual: (data.len() - magic_pos) as u64,
        });
    }

    let len_bytes: [u8; 4] = data[magic_pos + 4..header_end].try_into().expect("4 bytes");
    let payload_len_u64 = u32::from_be_bytes(len_bytes) as u64;

    let payload_start = header_end;
    let payload_end_u64 = (payload_start as u64)
        .checked_add(payload_len_u64)
        .expect("payload_start + u32 length fits in u64");
    if payload_end_u64 > data.len() as u64 {
        return Err(Rpcs3Error::TtyPayloadTooSmall {
            expected: payload_len_u64,
            actual: (data.len() - payload_start) as u64,
        });
    }
    let payload_end = payload_end_u64 as usize;
    let payload = &data[payload_start..payload_end];

    let mut total_needed: u64 = 0;
    for r in regions {
        total_needed =
            total_needed
                .checked_add(r.size)
                .ok_or_else(|| Rpcs3Error::TtyOffsetOverflow {
                    region_name: r.name.clone(),
                    size: r.size,
                })?;
    }
    if total_needed > payload_len_u64 {
        return Err(Rpcs3Error::TtyPayloadTooSmall {
            expected: total_needed,
            actual: payload_len_u64,
        });
    }

    let mut offset: u64 = 0;
    let mut result = Vec::with_capacity(regions.len());
    for region in regions {
        let region_end =
            offset
                .checked_add(region.size)
                .ok_or_else(|| Rpcs3Error::TtyOffsetOverflow {
                    region_name: region.name.clone(),
                    size: region.size,
                })?;
        // total_needed <= payload_len_u64 <= u32::MAX, so the per-region
        // accumulator stays within usize on all supported hosts.
        let lo = offset as usize;
        let hi = region_end as usize;
        result.push(NamedMemoryRegion {
            name: region.name.clone(),
            addr: region.guest_addr,
            data: payload[lo..hi].to_vec(),
        });
        offset = region_end;
    }
    Ok(result)
}

#[cfg(test)]
#[path = "tests/tty_tests.rs"]
mod tests;
