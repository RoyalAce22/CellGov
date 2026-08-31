//! Structural scan for LV2 sync-primitive handle slots in CG's
//! runtime data snapshot, plus the value-shape test for the handles
//! no struct layout locates.
//!
//! The layout scanners run against CG's runtime snapshot: the
//! `sys_lwmutex_t` preamble they match exists only after the title's
//! user-space init has run. Every other LV2 primitive hands the title
//! a bare `u32` id it stores anywhere. Nothing in the value marks it
//! as an id, so [`kernel_handle_pair`] tells a handle from a number
//! by the shape the minting allocator leaves on it.
//!
//! CellGov counts its own ids up from two separate allocators:
//!
//! - the shared kernel-id allocator, from [`FIRST_KERNEL_ID`];
//! - the lwmutex table's allocator, from 1.
//!
//! The CellGov-side shape test therefore forks on
//! [`KernelHandleKind::LwMutex`]. The per-kind base table recognises
//! the values in a comparison runner's memory dump. Neither scheme is
//! the console's.
//!
//! See [`crate::classify::DivergenceClass::SyncPrimitiveId`] for why
//! a differing handle is inert.

use std::collections::BTreeSet;
use std::ops::Range;

use cellgov_lv2::FIRST_KERNEL_ID;
use cellgov_mem::be::read_u32;

/// Byte offset of the `sleep_queue` field within `sys_lwmutex_t`.
/// The fields run lock_var (owner, waiter), attribute,
/// recursive_count, sleep_queue, pad.
pub const SLEEP_QUEUE_OFFSET: usize = 0x10;

/// Total size of `sys_lwmutex_t` (lock_var + attribute + recursive +
/// sleep_queue + pad).
pub const SYS_LWMUTEX_T_SIZE: usize = 0x20;

/// `lwmutex_free` sentinel. The user-space `sys_lwmutex_create`
/// wrapper writes it to `lock_var.owner` before it stores the kernel
/// handle.
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
/// The scan is 4-byte aligned and matches this `sys_lwmutex_t`
/// preamble:
///
/// ```text
/// +0x00: 0xffffffff   (lock_var.owner = lwmutex_free)
/// +0x04: 0x00000000   (lock_var.waiter)
/// +0x08: <attribute>  (one of eight valid recursive|protocol combos)
/// +0x0c: 0x00000000   (recursive_count = 0 at init)
/// +0x10: <sleep_queue>  (small int, the allocator id; THE claimed range)
/// +0x14: 0x00000000   (pad)
/// ```
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

/// Byte offset of the `lwcond_queue` field within `sys_lwcond_t`.
/// The fields run a pointer to the bound `sys_lwmutex_t`, then the
/// lwcond pseudo-id.
pub const LWCOND_QUEUE_OFFSET: usize = 0x4;

/// Total size of `sys_lwcond_t`.
pub const SYS_LWCOND_T_SIZE: usize = 0x8;

/// Ids CellGov's shared kernel-id allocator can have handed out in a
/// boot; titles create well under this many objects.
const CELLGOV_KERNEL_ID_MAX_PLAUSIBLE_COUNT: u32 = 0x0001_0000;

/// `true` when `w` is a handle CellGov's shared allocator minted.
fn is_cellgov_kernel_id(w: u32) -> bool {
    (FIRST_KERNEL_ID..FIRST_KERNEL_ID.saturating_add(CELLGOV_KERNEL_ID_MAX_PLAUSIBLE_COUNT))
        .contains(&w)
}

/// `true` when `w` is a handle CellGov's lwmutex allocator minted
/// (that allocator counts from 1).
fn is_cellgov_lwmutex_id(w: u32) -> bool {
    (1..SLEEP_QUEUE_MAX_PLAUSIBLE).contains(&w)
}

/// Walk `data` for `sys_lwcond_t` instances bound to an lwmutex the
/// lwmutex scan found, returning the guest-address range of each
/// instance's `lwcond_queue` field.
///
/// `lwmutex_slots` is [`find_sys_lwmutex_handle_slots`]'s output for
/// the same snapshot; an instance qualifies only when its `lwmutex`
/// pointer names one of those structs and its `lwcond_queue` holds a
/// CellGov kernel id, so a stray pointer-shaped word never matches.
///
/// # Panics
///
/// When a slot in `lwmutex_slots` starts below [`SLEEP_QUEUE_OFFSET`]:
/// no `sys_lwmutex_t` can hold it, so the slot did not come from the
/// lwmutex scan.
pub fn find_sys_lwcond_handle_slots(
    data: &[u8],
    data_base: u64,
    lwmutex_slots: &[Range<u64>],
) -> Vec<Range<u64>> {
    let lwmutex_bases: BTreeSet<u64> = lwmutex_slots
        .iter()
        .map(|slot| {
            slot.start
                .checked_sub(SLEEP_QUEUE_OFFSET as u64)
                .unwrap_or_else(|| {
                    panic!("lwmutex handle slot {slot:#x?} starts below its struct base")
                })
        })
        .collect();
    let mut out = Vec::new();
    if lwmutex_bases.is_empty() || data.len() < SYS_LWCOND_T_SIZE {
        return out;
    }
    let mut i = 0usize;
    while i + SYS_LWCOND_T_SIZE <= data.len() {
        let lwmutex_ptr = u64::from(read_u32(data, i));
        if lwmutex_bases.contains(&lwmutex_ptr) && is_cellgov_kernel_id(read_u32(data, i + 4)) {
            let slot_addr = data_base + i as u64 + LWCOND_QUEUE_OFFSET as u64;
            out.push(slot_addr..slot_addr + 4);
            i += SYS_LWCOND_T_SIZE;
            continue;
        }
        i += 4;
    }
    out
}

/// Which LV2 object a comparison runner's kernel id names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KernelHandleKind {
    /// `sys_mutex`.
    Mutex,
    /// `sys_cond`.
    Cond,
    /// `sys_rwlock`.
    RwLock,
    /// `sys_event_queue`.
    EventQueue,
    /// `sys_lwmutex` (kernel side of the lightweight mutex).
    LwMutex,
    /// `sys_semaphore`.
    Semaphore,
    /// `sys_lwcond` (kernel side of the lightweight cond).
    LwCond,
    /// `sys_event_flag`.
    EventFlag,
}

/// Per-kind id bases the comparison runner's allocator produces:
/// `id_base` in each RPCS3 `Emu/Cell/lv2/sys_*.h` object. Every kind
/// shares `lv2_obj`'s `id_step = 0x100` and `id_count = 8192`, and
/// the low byte holds that id manager's reuse counter (`sys_sync.h`
/// `lv2_obj::id_invl_range`).
///
/// The table lists only kinds whose id window lies outside memory a
/// guest can map on either runner. `sys_event_port` (base 0x0e) and
/// `sys_timer` (base 0x11) sit inside that runner's main and user
/// areas (`vm.cpp` `vm::init` block layout and `_find_map`). A heap
/// pointer there would pass as a handle and hide a real pointer
/// divergence, so those two kinds stay unclassified.
const RPCS3_ID_BASES: &[(u32, KernelHandleKind)] = &[
    (0x8500_0000, KernelHandleKind::Mutex),
    (0x8600_0000, KernelHandleKind::Cond),
    (0x8800_0000, KernelHandleKind::RwLock),
    (0x8d00_0000, KernelHandleKind::EventQueue),
    (0x9500_0000, KernelHandleKind::LwMutex),
    (0x9600_0000, KernelHandleKind::Semaphore),
    (0x9700_0000, KernelHandleKind::LwCond),
    (0x9800_0000, KernelHandleKind::EventFlag),
];
const RPCS3_ID_STEP: u32 = 0x100;
const RPCS3_ID_COUNT: u32 = 8192;

/// The kind of LV2 object `w` names, when the comparison runner's
/// allocator minted it.
pub fn rpcs3_kernel_handle_kind(w: u32) -> Option<KernelHandleKind> {
    let base = w & 0xff00_0000;
    let (_, kind) = RPCS3_ID_BASES.iter().find(|(b, _)| *b == base)?;
    let index = (w - base) / RPCS3_ID_STEP;
    (index < RPCS3_ID_COUNT).then_some(*kind)
}

/// The kind of LV2 object a 4-byte word holds when each allocator
/// minted one side, in either order.
///
/// Two like-shaped words are not a pair. The comparison is then
/// between like runners: one allocator produced both values, so a
/// difference is real.
pub fn kernel_handle_pair(a: u32, b: u32) -> Option<KernelHandleKind> {
    let pair = |rpcs3: u32, cellgov: u32| {
        let kind = rpcs3_kernel_handle_kind(rpcs3)?;
        let cellgov_shaped = match kind {
            KernelHandleKind::LwMutex => is_cellgov_lwmutex_id(cellgov),
            _ => is_cellgov_kernel_id(cellgov),
        };
        cellgov_shaped.then_some(kind)
    };
    pair(a, b).or_else(|| pair(b, a))
}

#[cfg(test)]
#[path = "tests/sync_primitive_scan_tests.rs"]
mod tests;
