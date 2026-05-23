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
mod tests {
    use super::*;
    use cellgov_mem::{GuestMemory, PageSize, Region};
    use cellgov_time::Budget;

    fn rt_with_layout() -> Runtime {
        let mem = GuestMemory::from_regions(vec![
            Region::new(0, 0x4000_0000, "main", PageSize::Page64K),
            Region::new(0xD000_0000, 0x0001_0000, "stack", PageSize::Page4K),
        ])
        .unwrap();
        Runtime::new(mem, Budget::new(1), 100)
    }

    #[test]
    fn region_label_at_names_stack_region() {
        let rt = rt_with_layout();
        assert_eq!(region_label_at(&rt, 0xD000_FFF0, 4), "stack");
    }

    #[test]
    fn region_label_at_names_main_region() {
        let rt = rt_with_layout();
        assert_eq!(region_label_at(&rt, 0x0010_0000, 4), "main");
    }

    #[test]
    fn region_label_at_unmapped_addr_is_not_misattributed() {
        let rt = rt_with_layout();
        assert_eq!(region_label_at(&rt, 0x8000_0000, 4), "<unmapped>");
    }

    #[test]
    fn longest_readable_prefix_returns_none_on_zero_length() {
        let rt = rt_with_layout();
        assert!(longest_readable_prefix(rt.memory(), 0, 0).is_none());
    }

    #[test]
    fn longest_readable_prefix_returns_none_for_entirely_unmapped_buffer() {
        let rt = rt_with_layout();
        assert!(longest_readable_prefix(rt.memory(), 0x8000_0000, 64).is_none());
    }

    #[test]
    fn longest_readable_prefix_finds_region_boundary_exactly() {
        let rt = rt_with_layout();
        assert!(
            longest_readable_prefix(rt.memory(), 0x4000_0000, 1).is_none(),
            "precondition: nothing readable at main's end"
        );
        let buf = 0x4000_0000 - 16;
        let (n, bytes) = longest_readable_prefix(rt.memory(), buf, 64).expect("some prefix");
        assert_eq!(n, 16);
        assert_eq!(bytes.len(), 16);
    }

    #[test]
    fn longest_readable_prefix_returns_full_len_when_fully_mapped() {
        let rt = rt_with_layout();
        let (n, bytes) = longest_readable_prefix(rt.memory(), 0x0010_0000, 64)
            .expect("fully readable should return Some");
        assert_eq!(n, 64);
        assert_eq!(bytes.len(), 64);
    }

    #[test]
    fn longest_readable_prefix_single_byte_boundary() {
        let rt = rt_with_layout();
        let buf = 0x4000_0000 - 1;
        let (n, _bytes) = longest_readable_prefix(rt.memory(), buf, 2).expect("single-byte prefix");
        assert_eq!(n, 1);
    }
}
