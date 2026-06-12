//! Shared-memory handle table backing 332 / 362 / 334.
//!
//! # Cross-module contract
//!
//! 332 / 362 mint a fresh `mem_id` and record `(size, align)` here;
//! 334 looks the entry up and emits a pending region-install request
//! the runtime drains after dispatch. Dispatch handlers live in
//! [`crate::host::dispatch_route::unsupported_arms`]; this module is
//! data only.

use std::collections::BTreeMap;

/// One shared-memory handle recorded by 332 or 362.
///
/// `align` is the page granule derived from the caller's `flags` per
/// RPCS3's `sys_mmapper.cpp` granule resolution.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct MmapperHandle {
    pub size: u32,
    pub align: u32,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct MmapperHandleTable {
    handles: BTreeMap<u32, MmapperHandle>,
}

impl MmapperHandleTable {
    pub(crate) fn new() -> Self {
        Self::default()
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

    /// `None` matches RPCS3's CELL_ESRCH path when `idm::get` fails.
    pub(crate) fn get(&self, mem_id: u32) -> Option<MmapperHandle> {
        self.handles.get(&mem_id).copied()
    }
}

/// Pending region-install request emitted by 334 and drained by the
/// runtime after `Lv2Host::dispatch` returns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PendingRegionInstall {
    pub addr: u64,
    pub size: usize,
}

/// Deterministic boot-state writes applied when the shm registered
/// under `shm_ipc_key` is first mapped into guest memory (334 / 337).
///
/// Models system state an external firmware producer would have
/// established before the title ran; CellGov encodes it as data, not
/// as cross-process communication.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SystemStateSeed {
    /// Must match a key later passed to a keyed 332.
    pub shm_ipc_key: u64,
    /// `(offset, bytes)` big-endian writes relative to the mapped
    /// base. Every write must land inside the registered handle's
    /// size.
    pub writes: Vec<(u32, Vec<u8>)>,
}
