//! Small read-only diagnostic helpers: ASCII byte preview, raw-word
//! fetch at a PC, region label lookup, longest-readable-prefix probe,
//! and HLE-index formatter. Shared across the fault / exit / max-step
//! formatters in sibling submodules.

use cellgov_core::Runtime;

pub(in crate::game) fn ascii_safe_preview(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|&b| {
            if (0x20..=0x7E).contains(&b) {
                b as char
            } else {
                '.'
            }
        })
        .collect()
}

pub(in crate::game) fn fetch_raw_at(rt: &Runtime, pc: u64) -> Option<u32> {
    let range = cellgov_mem::ByteRange::new(cellgov_mem::GuestAddr::new(pc), 4)?;
    let b = rt.memory().read(range)?;
    Some(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
}

/// `len` must match the caller's read width; querying with `len=1` mislabels
/// a PC 1-3 bytes before a boundary as mapped when a 4-byte fetch would fail.
pub(in crate::game) fn region_label_at(rt: &Runtime, addr: u64, len: u64) -> &'static str {
    rt.memory()
        .containing_region(addr, len)
        .map(|r| r.label())
        .unwrap_or("<unmapped>")
}

/// Longest readable prefix of `[buf, buf+len)` via O(log len) probes.
pub(in crate::game) fn longest_readable_prefix(
    mem: &cellgov_mem::GuestMemory,
    buf: u64,
    len: u64,
) -> Option<(u64, Vec<u8>)> {
    if len == 0 {
        return None;
    }
    let mut lo = 0u64;
    let mut hi = len;
    while lo < hi {
        let mid = lo + (hi - lo).div_ceil(2);
        let hit = cellgov_mem::ByteRange::new(cellgov_mem::GuestAddr::new(buf), mid)
            .and_then(|r| mem.read(r))
            .is_some();
        if hit {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    if lo == 0 {
        return None;
    }
    let r = cellgov_mem::ByteRange::new(cellgov_mem::GuestAddr::new(buf), lo)?;
    let bytes = mem.read(r)?.to_vec();
    Some((lo, bytes))
}

pub(in crate::game) fn format_hle_idx(idx: u32) -> String {
    format!("<hle-idx-{idx}>")
}

#[cfg(test)]
#[path = "tests/helpers_tests.rs"]
mod tests;
