//! Big-endian scalar reads from committed guest memory, shared by the
//! unsupported-arm families.

use crate::host::Lv2Runtime;

/// Big-endian u32 at `addr`, or `None` when the range is unmapped.
pub(super) fn read_be_u32(rt: &dyn Lv2Runtime, addr: u64) -> Option<u32> {
    let b = rt.read_committed(addr, 4)?;
    Some(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
}

/// Big-endian u64 at `addr`, or `None` when the range is unmapped.
pub(super) fn read_be_u64(rt: &dyn Lv2Runtime, addr: u64) -> Option<u64> {
    let b = rt.read_committed(addr, 8)?;
    Some(u64::from_be_bytes([
        b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7],
    ]))
}
