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
//! CellGov mints its ids in the console's form: the class in the top
//! byte, a generation and a slot below it, and a zero low byte
//! (`cellgov_ps3_abi::lv2::kernel_id`). The comparison runner puts the
//! same class in the top byte over its own index and reuse counter, so
//! the two shapes overlap: a word pair whose sides carry one class is
//! a handle pair, whichever runner minted each side.
//!
//! See [`crate::classify::DivergenceClass::SyncPrimitiveId`] for why
//! a differing handle is inert.

use std::collections::BTreeSet;
use std::ops::Range;

use cellgov_mem::be::read_u32;
use cellgov_ps3_abi::lv2::kernel_id::{class_of, CLASS_SHIFT, SLOT_SHIFT};
use cellgov_ps3_abi::lv2::process::{
    ProcessObjectClassId, SYS_COND_OBJECT, SYS_EVENT_FLAG_OBJECT, SYS_EVENT_QUEUE_OBJECT,
    SYS_LWCOND_OBJECT, SYS_LWMUTEX_OBJECT, SYS_MUTEX_OBJECT, SYS_RWLOCK_OBJECT,
    SYS_SEMAPHORE_OBJECT,
};

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
/// +0x10: <sleep_queue>  (a CellGov lwmutex id; THE claimed range)
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
                && is_cellgov_kernel_id(w4, SYS_LWMUTEX_OBJECT)
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

/// `true` when `w` has the shape of an id of `class` CellGov's kernel-id
/// table mints: that class in the top byte and a zero low byte.
fn is_cellgov_kernel_id(w: u32, class: ProcessObjectClassId) -> bool {
    class_of(w) == class && w & ((1 << SLOT_SHIFT) - 1) == 0
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
        if lwmutex_bases.contains(&lwmutex_ptr)
            && is_cellgov_kernel_id(read_u32(data, i + 4), SYS_LWCOND_OBJECT)
        {
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

/// The sync-primitive classes this recogniser reads out of a
/// comparison runner's dump, keyed by the [`ProcessObjectClassId`]
/// each one's ids carry in their top byte.
///
/// The table lists only the classes whose id window falls outside the
/// addresses guest allocations come from. `SYS_EVENT_PORT_OBJECT`
/// (0x0e) and `SYS_TIMER_OBJECT` (0x11) put theirs at the bottom of
/// the address space. A heap pointer there would pass as a handle and
/// hide a real pointer divergence, so those two stay unclassified.
const RUNNER_ID_CLASSES: &[(ProcessObjectClassId, KernelHandleKind)] = &[
    (SYS_MUTEX_OBJECT, KernelHandleKind::Mutex),
    (SYS_COND_OBJECT, KernelHandleKind::Cond),
    (SYS_RWLOCK_OBJECT, KernelHandleKind::RwLock),
    (SYS_EVENT_QUEUE_OBJECT, KernelHandleKind::EventQueue),
    (SYS_LWMUTEX_OBJECT, KernelHandleKind::LwMutex),
    (SYS_SEMAPHORE_OBJECT, KernelHandleKind::Semaphore),
    (SYS_LWCOND_OBJECT, KernelHandleKind::LwCond),
    (SYS_EVENT_FLAG_OBJECT, KernelHandleKind::EventFlag),
];

/// Bits below the class id in a handle the comparison runner minted.
const RUNNER_ID_CLASS_SHIFT: u32 = CLASS_SHIFT;

/// Stride between consecutive ids of one class in that runner's
/// dumps. The bytes under it hold its allocator's reuse counter.
const RUNNER_ID_STEP: u32 = 0x100;

/// Ids that runner's allocator hands out per class.
const RUNNER_ID_COUNT: u32 = 8192;

/// The kind of LV2 object `w` names, when the comparison runner's
/// allocator minted it.
///
/// The stride and the per-class count belong to that runner. Only the
/// class id in the top byte is a constant CellGov holds itself.
pub fn runner_kernel_handle_kind(w: u32) -> Option<KernelHandleKind> {
    let class = w >> RUNNER_ID_CLASS_SHIFT;
    let (_, kind) = RUNNER_ID_CLASSES.iter().find(|(c, _)| *c == class)?;
    let index = (w & ((1 << RUNNER_ID_CLASS_SHIFT) - 1)) / RUNNER_ID_STEP;
    (index < RUNNER_ID_COUNT).then_some(*kind)
}

/// The kind of LV2 object a 4-byte word holds when one side has the
/// comparison runner's shape and the other CellGov's, for one class,
/// in either order.
///
/// The two shapes overlap, so two ids of one class pair whichever
/// runner minted each side.
pub fn kernel_handle_pair(a: u32, b: u32) -> Option<KernelHandleKind> {
    let pair = |runner: u32, cellgov: u32| {
        let kind = runner_kernel_handle_kind(runner)?;
        is_cellgov_kernel_id(cellgov, class_of(runner)).then_some(kind)
    };
    pair(a, b).or_else(|| pair(b, a))
}

#[cfg(test)]
#[path = "tests/sync_primitive_scan_tests.rs"]
mod tests;
