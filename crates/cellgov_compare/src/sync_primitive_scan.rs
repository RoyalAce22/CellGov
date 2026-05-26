//! Structural scan for LV2 sync-primitive handle slots in CG's
//! runtime data snapshot.
//!
//! The user-space `sys_lwmutex_t` carries a `sleep_queue` field at
//! +0x10 that the firmware sysPrxForUser wrapper fills with the
//! kernel-allocated lwmutex id after `_sys_lwmutex_create` returns.
//! Two runners' id allocators produce different bytes for the same
//! logical object -- a non-semantic divergence, equivalence-up-to-
//! kernel-id-allocator. See [`crate::classify::DivergenceClass::SyncPrimitiveId`]
//! for the inertness warrant.
//!
//! The scanner walks a runtime data segment for the `sys_lwmutex_t`
//! preamble and yields the 4-byte `sleep_queue` range per match.
//! It runs against CG's snapshot, not the EBOOT, because the
//! preamble's `lwmutex_free` sentinel and `attribute` value are only
//! present after the title's user-space init has run.

use std::ops::Range;

/// Byte offset of the `sleep_queue` field within `sys_lwmutex_t`.
/// Matches `tools/rpcs3-src/rpcs3/Emu/Cell/lv2/sys_lwmutex.h:50`.
pub const SLEEP_QUEUE_OFFSET: usize = 0x10;

/// Total size of `sys_lwmutex_t` (lock_var + attribute + recursive +
/// sleep_queue + pad).
pub const SYS_LWMUTEX_T_SIZE: usize = 0x20;

/// `lwmutex_free` sentinel: written to `lock_var.owner` by the
/// user-space `sys_lwmutex_create` wrapper before the kernel handle
/// is stored. Matches `sys_lwmutex.h:21`.
const LWMUTEX_FREE: u32 = 0xffff_ffff;

/// Read a big-endian u32 from a slice at the given byte offset.
fn read_u32_be(data: &[u8], off: usize) -> u32 {
    u32::from_be_bytes([data[off], data[off + 1], data[off + 2], data[off + 3]])
}

/// Validate that `attr` is a plausible `sys_lwmutex_attribute_t::recursive | protocol`.
/// `recursive` is one of 0x10 (SYS_SYNC_RECURSIVE) / 0x20 (SYS_SYNC_NOT_RECURSIVE);
/// `protocol` is 1..=4 (FIFO / PRIORITY / PRIORITY_INHERIT / RETRY).
/// Eight valid combinations.
fn is_valid_lwmutex_attribute(attr: u32) -> bool {
    let recursive = attr & 0xf0;
    let protocol = attr & 0x0f;
    matches!(recursive, 0x10 | 0x20) && matches!(protocol, 1..=4)
}

/// Upper bound on the `sleep_queue` field's plausible value when scanning
/// a CG snapshot. CG's monotonic id allocator starts at 1; titles create
/// well under 10,000 sync primitives during boot. A loose cap keeps the
/// signature strict enough to reject random data while not depending on
/// a tight bound.
const SLEEP_QUEUE_MAX_PLAUSIBLE: u32 = 0x0001_0000;

/// Walk `data` for `sys_lwmutex_t` instances and return the guest-
/// address range of each instance's `sleep_queue` field.
///
/// `data_base` is the guest address of `data[0]` (the region's `addr`).
///
/// The scan is 4-byte aligned and matches the `sys_lwmutex_t` preamble:
///
/// ```text
/// +0x00: 0xffffffff   (lock_var.owner = lwmutex_free)
/// +0x04: 0x00000000   (lock_var.waiter)
/// +0x08: <attribute>  (one of eight valid recursive|protocol combos)
/// +0x0c: 0x00000000   (recursive_count = 0 at init)
/// +0x10: <sleep_queue>  (small int, the allocator id; THE claimed range)
/// +0x14: 0x00000000   (pad)
/// ```
///
/// The sleep_queue field is validated to be 0 (uninitialized) or
/// less than [`SLEEP_QUEUE_MAX_PLAUSIBLE`]. Pad at +0x14 must be
/// zero. Together these constraints make false positives on random
/// data vanishingly unlikely while admitting every well-formed
/// lwmutex slot the title initializes.
pub fn find_sys_lwmutex_handle_slots(data: &[u8], data_base: u64) -> Vec<Range<u64>> {
    let mut out = Vec::new();
    if data.len() < SYS_LWMUTEX_T_SIZE {
        return out;
    }
    let mut i = 0usize;
    while i + SYS_LWMUTEX_T_SIZE <= data.len() {
        let w0 = read_u32_be(data, i);
        if w0 == LWMUTEX_FREE {
            let w1 = read_u32_be(data, i + 0x04);
            let w2 = read_u32_be(data, i + 0x08);
            let w3 = read_u32_be(data, i + 0x0c);
            let w4 = read_u32_be(data, i + 0x10);
            let w5 = read_u32_be(data, i + 0x14);
            let preamble_match = w1 == 0
                && is_valid_lwmutex_attribute(w2)
                && w3 == 0
                && w4 < SLEEP_QUEUE_MAX_PLAUSIBLE
                && w5 == 0;
            if preamble_match {
                let slot_addr = data_base + i as u64 + SLEEP_QUEUE_OFFSET as u64;
                out.push(slot_addr..slot_addr + 4);
                i += SYS_LWMUTEX_T_SIZE;
                continue;
            }
        }
        i += 4;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn emit_lwmutex(buf: &mut Vec<u8>, attribute: u32, sleep_queue: u32) {
        buf.extend_from_slice(&LWMUTEX_FREE.to_be_bytes());
        buf.extend_from_slice(&0u32.to_be_bytes());
        buf.extend_from_slice(&attribute.to_be_bytes());
        buf.extend_from_slice(&0u32.to_be_bytes());
        buf.extend_from_slice(&sleep_queue.to_be_bytes());
        buf.extend_from_slice(&0u32.to_be_bytes());
        buf.extend_from_slice(&0u32.to_be_bytes());
        buf.extend_from_slice(&0u32.to_be_bytes());
    }

    #[test]
    fn finds_one_lwmutex_at_data_base() {
        let mut data = Vec::new();
        emit_lwmutex(&mut data, 0x22, 13);
        let ranges = find_sys_lwmutex_handle_slots(&data, 0x860000);
        assert_eq!(ranges, vec![(0x860000 + 0x10)..(0x860000 + 0x14)]);
    }

    #[test]
    fn finds_multiple_separated_by_padding() {
        // First struct at offset 0x00..0x20 (sleep_queue at 0x10).
        // 16 bytes of padding at 0x20..0x30.
        // Second struct at offset 0x30..0x50 (sleep_queue at 0x40).
        let mut data = Vec::new();
        emit_lwmutex(&mut data, 0x22, 13);
        data.extend_from_slice(&[0u8; 16]);
        emit_lwmutex(&mut data, 0x21, 14);
        let ranges = find_sys_lwmutex_handle_slots(&data, 0x860000);
        assert_eq!(ranges, vec![0x860010..0x860014, 0x860040..0x860044]);
    }

    #[test]
    fn rejects_wrong_sentinel() {
        let mut data = Vec::new();
        // Write a valid struct minus the sentinel.
        data.extend_from_slice(&[0u8; 4]); // owner = 0 (not lwmutex_free)
        data.extend_from_slice(&0u32.to_be_bytes());
        data.extend_from_slice(&0x22u32.to_be_bytes());
        data.extend_from_slice(&0u32.to_be_bytes());
        data.extend_from_slice(&13u32.to_be_bytes());
        data.extend_from_slice(&[0u8; 12]);
        let ranges = find_sys_lwmutex_handle_slots(&data, 0x860000);
        assert!(ranges.is_empty());
    }

    #[test]
    fn rejects_invalid_attribute() {
        let mut data = Vec::new();
        emit_lwmutex(&mut data, 0xdeadbeef, 13);
        let ranges = find_sys_lwmutex_handle_slots(&data, 0x860000);
        assert!(ranges.is_empty());
    }

    #[test]
    fn rejects_nonzero_pad() {
        let mut data = Vec::new();
        data.extend_from_slice(&LWMUTEX_FREE.to_be_bytes());
        data.extend_from_slice(&0u32.to_be_bytes());
        data.extend_from_slice(&0x22u32.to_be_bytes());
        data.extend_from_slice(&0u32.to_be_bytes());
        data.extend_from_slice(&13u32.to_be_bytes());
        data.extend_from_slice(&0xCAFEBABEu32.to_be_bytes()); // pad != 0
        data.extend_from_slice(&[0u8; 8]);
        let ranges = find_sys_lwmutex_handle_slots(&data, 0x860000);
        assert!(ranges.is_empty());
    }

    #[test]
    fn rejects_large_sleep_queue_value() {
        let mut data = Vec::new();
        // sleep_queue = 0x95002000 (RPCS3-style id, larger than CG's plausible cap).
        emit_lwmutex(&mut data, 0x22, 0x95002000);
        let ranges = find_sys_lwmutex_handle_slots(&data, 0x860000);
        // CG's snapshot should never carry an id this large.
        assert!(ranges.is_empty());
    }

    #[test]
    fn empty_or_too_small_data_returns_empty() {
        let ranges = find_sys_lwmutex_handle_slots(&[], 0x860000);
        assert!(ranges.is_empty());
        let ranges = find_sys_lwmutex_handle_slots(&[0u8; 8], 0x860000);
        assert!(ranges.is_empty());
    }

    #[test]
    fn accepts_all_eight_valid_attribute_combos() {
        for attr in [0x11, 0x12, 0x13, 0x14, 0x21, 0x22, 0x23, 0x24] {
            let mut data = Vec::new();
            emit_lwmutex(&mut data, attr, 1);
            let ranges = find_sys_lwmutex_handle_slots(&data, 0x860000);
            assert_eq!(ranges.len(), 1, "attr 0x{attr:x} rejected unexpectedly");
        }
    }
}
