//! Shared-memory handle table backing 332 / 362 / 334.
//!
//! # Cross-module contract
//!
//! 332 / 362 mint a fresh `mem_id` and record `(size, align)` here;
//! 334 looks the entry up and emits a pending region-install request
//! the runtime drains after dispatch, and 335 emits the matching
//! removal. Dispatch handlers live in
//! `host::dispatch_route::unsupported_arms::memory`; this module is
//! data only.

use cellgov_mem::lanes::{source, LaneMap, LaneValue, ObjectLanes};

/// One shared-memory handle recorded by 332 or 362.
///
/// `align` is the page granule the caller's `flags` name: libaudio.prx
/// rounds its request up to 64 KiB and passes 0x200 in the same
/// `sys_mmapper_allocate_shared_memory` call.
///
/// `stuck_maps` counts the maps a 337 refused with `CELL_EFAULT` after
/// after it took them: the `lv2_mmapper_unmap` capture shows such a
/// segment answering `CELL_EBUSY` to every later free, with no window
/// left to unmap.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct MmapperHandle {
    pub size: u32,
    pub align: u32,
    pub stuck_maps: u32,
}

impl LaneValue for MmapperHandle {
    fn lanes(&self, lanes: &mut ObjectLanes) {
        lanes.lane(1, 0, u64::from(self.size));
        lanes.lane(2, 0, u64::from(self.align));
        if self.stuck_maps != 0 {
            lanes.lane(3, 0, u64::from(self.stuck_maps));
        }
    }
}

/// The bytes a segment held when 335 unmapped its last window.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ParkedContents(Vec<u8>);

impl LaneValue for ParkedContents {
    fn lanes(&self, lanes: &mut ObjectLanes) {
        lanes.bytes(1, &[], &self.0);
    }
}

#[derive(Debug, Clone)]
pub(crate) struct MmapperHandleTable {
    handles: LaneMap<u32, MmapperHandle>,
    /// The contents of each segment an unmap left with no window, until
    /// a map takes them back: the `lv2_mmapper_remap` capture shows a
    /// word written through one window read back through a later map of
    /// the segment, by 334 at another address and by 337.
    parked: LaneMap<u32, ParkedContents>,
}

impl MmapperHandleTable {
    pub(crate) fn new() -> Self {
        Self {
            handles: LaneMap::new(source::MMAPPER_HANDLE, u64::from),
            parked: LaneMap::new(source::MMAPPER_PARKED, u64::from),
        }
    }

    /// Keep `bytes` as `mem_id`'s contents; `false` when no handle
    /// names `mem_id`.
    pub(crate) fn park(&mut self, mem_id: u32, bytes: Vec<u8>) -> bool {
        if !self.handles.contains_key(mem_id) {
            return false;
        }
        self.parked.insert(mem_id, ParkedContents(bytes));
        true
    }

    /// Take back the contents [`Self::park`] kept for `mem_id`.
    pub(crate) fn take_parked(&mut self, mem_id: u32) -> Option<Vec<u8>> {
        self.parked.remove(mem_id).map(|p| p.0)
    }

    /// Caller (332 / 362 dispatch) owns `mem_id` allocation via
    /// `Lv2Host::alloc_id`.
    pub(crate) fn insert(&mut self, mem_id: u32, handle: MmapperHandle) {
        let prior = self.handles.insert(mem_id, handle);
        debug_assert!(
            prior.is_none(),
            "mem_id {mem_id:#x} already in handle table; alloc_id collision",
        );
    }

    /// Removes the handle `mem_id` names; `None` when no create minted
    /// it or a free already removed it.
    pub(crate) fn remove(&mut self, mem_id: u32) -> Option<MmapperHandle> {
        self.parked.remove(mem_id);
        self.handles.remove(mem_id)
    }

    /// `None` is the caller's CELL_ESRCH arm: a `mem_id` no create
    /// minted names nothing.
    pub(crate) fn get(&self, mem_id: u32) -> Option<MmapperHandle> {
        self.handles.get(mem_id).copied()
    }

    /// Count one map of `mem_id` that no window holds; see
    /// [`MmapperHandle::stuck_maps`].
    pub(crate) fn note_stuck_map(&mut self, mem_id: u32) {
        if let Some(mut handle) = self.handles.get_mut(mem_id) {
            handle.stuck_maps = handle.stuck_maps.saturating_add(1);
        }
    }

    #[cfg(test)]
    pub(crate) fn is_empty(&self) -> bool {
        self.handles.is_empty()
    }

    /// The table's partial of the sync-state sum.
    pub(crate) fn sync_partial(&self) -> u128 {
        self.handles.partial().wrapping_add(self.parked.partial())
    }

    /// [`Self::sync_partial`] computed from every entry.
    pub(crate) fn sync_partial_from_scratch(&self) -> u128 {
        self.handles
            .partial_from_scratch()
            .wrapping_add(self.parked.partial_from_scratch())
    }
}

/// An address area `sys_mmapper_allocate_address` (330) reserved: its
/// size and the page size its `flags` named.
///
/// The `lv2_mmapper_map` and `lv2_mmapper_map_64k` captures show the page
/// size deciding a map: the base a 334 names must be a multiple of it,
/// and a segment whose own granule is finer than it is refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct MmapperArea {
    pub size: u32,
    pub page: u32,
}

impl LaneValue for MmapperArea {
    fn lanes(&self, lanes: &mut ObjectLanes) {
        lanes.lane(1, 0, u64::from(self.size));
        lanes.lane(2, 0, u64::from(self.page));
    }
}

/// The lane object of a `(pid, base)` key: the pid in the high word.
/// Every pid is nonzero, so the object needs more than the 32 packed
/// object bits and each entry's lanes take the mixer term.
pub(crate) fn pid_base_object((pid, base): (u32, u32)) -> u64 {
    u64::from(pid) << 32 | u64::from(base)
}

/// A window 334 or 337 mapped and no 335 unmapped: the segment it views
/// and the window's length, the segment's size.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct MmapperWindow {
    pub mem_id: u32,
    pub size: u32,
}

impl LaneValue for MmapperWindow {
    fn lanes(&self, lanes: &mut ObjectLanes) {
        lanes.lane(1, 0, u64::from(self.mem_id));
        lanes.lane(2, 0, u64::from(self.size));
    }
}

/// A region-removal request 335 emits; the runtime drains it after
/// `Lv2Host::dispatch` returns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PendingRegionRemoval {
    pub addr: u64,
    pub size: usize,
    /// The segment the window viewed, which keeps the window's bytes.
    pub mem_id: u32,
    /// The IPC key of the segment the window viewed, as on
    /// [`PendingRegionInstall::ipc_key`].
    pub ipc_key: Option<u64>,
}

/// Pending region-install request emitted by 334 / 337 and drained by
/// the runtime after `Lv2Host::dispatch` returns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PendingRegionInstall {
    pub addr: u64,
    pub size: usize,
    /// The segment the window views; its parked contents fill the
    /// window.
    pub mem_id: u32,
    /// The IPC key the mapped `mem_id` was registered under by a
    /// keyed 332, `None` for keyless handles. The runtime uses it to
    /// keep views of one keyed segment coherent across address
    /// spaces.
    pub ipc_key: Option<u64>,
}

/// Deterministic boot-state writes applied when the shm registered
/// under `shm_ipc_key` is first mapped into guest memory (334 / 337).
///
/// Models system state an external firmware producer would have
/// established before the title ran.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SystemStateSeed {
    /// Must match a key later passed to a keyed 332.
    pub shm_ipc_key: u64,
    /// `(offset, bytes)` big-endian writes relative to the mapped
    /// base. Every write must land inside the registered handle's
    /// size.
    pub writes: Vec<(u32, Vec<u8>)>,
}

#[cfg(test)]
#[path = "tests/mmapper_allocate_console_tests.rs"]
mod console_tests;

#[cfg(test)]
#[path = "tests/mmapper_map_console_tests.rs"]
mod map_console_tests;

#[cfg(test)]
#[path = "tests/mmapper_unmap_console_tests.rs"]
mod unmap_console_tests;

#[cfg(test)]
#[path = "tests/mmapper_process_tests.rs"]
mod process_tests;
