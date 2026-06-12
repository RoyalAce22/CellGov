//! Structural scan for LV2 sync-primitive handle slots in CG's
//! runtime data snapshot.
//!
//! The user-space `sys_lwmutex_t` carries a `sleep_queue` field at
//! +0x10 that the firmware sysPrxForUser wrapper fills with the
//! kernel-allocated lwmutex id after `_sys_lwmutex_create` returns.
//! Two runners' id allocators produce different bytes for the same
//! logical object. See
//! [`crate::classify::DivergenceClass::SyncPrimitiveId`] for the
//! inertness warrant.
//!
//! The scanner walks a runtime data segment for the `sys_lwmutex_t`
//! preamble and yields the 4-byte `sleep_queue` range per match.
//! It runs against CG's snapshot, not the EBOOT: the preamble's
//! `lwmutex_free` sentinel and `attribute` value are only present
//! after the title's user-space init has run.

use cellgov_mem::be::read_u32;
use std::ops::Range;

/// Byte offset of the `sleep_queue` field within `sys_lwmutex_t`.
/// Matches RPCS3's `sys_lwmutex.h` struct layout.
pub const SLEEP_QUEUE_OFFSET: usize = 0x10;

/// Total size of `sys_lwmutex_t` (lock_var + attribute + recursive +
/// sleep_queue + pad).
pub const SYS_LWMUTEX_T_SIZE: usize = 0x20;

/// `lwmutex_free` sentinel: written to `lock_var.owner` by the
/// user-space `sys_lwmutex_create` wrapper before the kernel handle
/// is stored. Matches RPCS3's `sys_lwmutex.h` constant.
const LWMUTEX_FREE: u32 = 0xffff_ffff;

/// Validate that `attr` is a plausible `sys_lwmutex_attribute_t::recursive | protocol`.
/// `recursive` is one of 0x10 (SYS_SYNC_RECURSIVE) / 0x20 (SYS_SYNC_NOT_RECURSIVE);
/// `protocol` is 1..=4 (FIFO / PRIORITY / PRIORITY_INHERIT / RETRY).
/// Eight valid combinations.
fn is_valid_lwmutex_attribute(attr: u32) -> bool {
    let recursive = attr & 0xf0;
    let protocol = attr & 0x0f;
    matches!(recursive, 0x10 | 0x20) && matches!(protocol, 1..=4)
}

/// Upper bound on the `sleep_queue` field's plausible value in a CG
/// snapshot. CG's monotonic id allocator starts at 1; titles create
/// well under 10,000 sync primitives during boot.
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
/// below an internal plausibility cap, and the pad at +0x14 must
/// be zero. Together these constraints make false positives on
/// random data vanishingly unlikely while admitting every
/// well-formed lwmutex slot the title initializes.
pub fn find_sys_lwmutex_handle_slots(data: &[u8], data_base: u64) -> Vec<Range<u64>> {
    let mut out = Vec::new();
    if data.len() < SYS_LWMUTEX_T_SIZE {
        return out;
    }
    let mut i = 0usize;
    while i + SYS_LWMUTEX_T_SIZE <= data.len() {
        let w0 = read_u32(data, i);
        if w0 == LWMUTEX_FREE {
            let w1 = read_u32(data, i + 0x04);
            let w2 = read_u32(data, i + 0x08);
            let w3 = read_u32(data, i + 0x0c);
            let w4 = read_u32(data, i + 0x10);
            let w5 = read_u32(data, i + 0x14);
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
#[path = "tests/sync_primitive_scan_tests.rs"]
mod tests;
