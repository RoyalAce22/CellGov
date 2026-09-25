//! Shared-memory handle table backing 332 / 362 / 334.
//!
//! # Cross-module contract
//!
//! 332 / 362 mint a fresh `mem_id` and record `(size, align)` here;
//! 334 looks the entry up and emits a pending region-install request
//! the runtime drains after dispatch. Dispatch handlers live in
//! `host::dispatch_route::unsupported_arms::memory`; this module is
//! data only.

use cellgov_mem::lanes::{source, LaneMap, LaneValue, ObjectLanes};

/// One shared-memory handle recorded by 332 or 362.
///
/// `align` is the page granule the caller's `flags` name: libaudio.prx
/// rounds its request up to 64 KiB and passes 0x200 in the same
/// `sys_mmapper_allocate_shared_memory` call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct MmapperHandle {
    pub size: u32,
    pub align: u32,
}

impl LaneValue for MmapperHandle {
    fn lanes(&self, lanes: &mut ObjectLanes) {
        lanes.lane(1, 0, u64::from(self.size));
        lanes.lane(2, 0, u64::from(self.align));
    }
}

#[derive(Debug, Clone)]
pub(crate) struct MmapperHandleTable {
    handles: LaneMap<u32, MmapperHandle>,
}

impl MmapperHandleTable {
    pub(crate) fn new() -> Self {
        Self {
            handles: LaneMap::new(source::MMAPPER_HANDLE, u64::from),
        }
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

    /// `None` is the caller's CELL_ESRCH arm: a `mem_id` no create
    /// minted names nothing.
    pub(crate) fn get(&self, mem_id: u32) -> Option<MmapperHandle> {
        self.handles.get(mem_id).copied()
    }

    #[cfg(test)]
    pub(crate) fn is_empty(&self) -> bool {
        self.handles.is_empty()
    }

    /// The table's partial of the sync-state sum.
    pub(crate) fn sync_partial(&self) -> u128 {
        self.handles.partial()
    }

    /// [`Self::sync_partial`] computed from every entry.
    pub(crate) fn sync_partial_from_scratch(&self) -> u128 {
        self.handles.partial_from_scratch()
    }
}

/// Pending region-install request emitted by 334 / 337 and drained by
/// the runtime after `Lv2Host::dispatch` returns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PendingRegionInstall {
    pub addr: u64,
    pub size: usize,
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
