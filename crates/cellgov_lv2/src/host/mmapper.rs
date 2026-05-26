//! Shared-memory handle table backing `sys_mmapper_allocate_shared_memory`
//! (332), `sys_mmapper_allocate_shared_memory_from_container` (362),
//! and `sys_mmapper_map_shared_memory` (334).
//!
//! 332 / 362 mint a fresh `mem_id` and record `(size, align)` here;
//! 334 looks the entry up and emits a pending region-install request
//! the runtime drains after dispatch. Behaviour (the dispatch handlers)
//! lives in [`crate::host::dispatch_route::unsupported_arms`]; this
//! module is data only.

use std::collections::BTreeMap;

/// One shared-memory handle as recorded by `sys_mmapper_allocate_shared_memory`
/// (332) or `sys_mmapper_allocate_shared_memory_from_container` (362).
///
/// `size` is the byte length the title later expects 334 to back; `align`
/// is the page granule derived from the caller's `flags` per
/// `tools/rpcs3-src/rpcs3/Emu/Cell/lv2/sys_mmapper.cpp:232`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct MmapperHandle {
    pub size: u32,
    pub align: u32,
}

/// Handle table keyed on `mem_id`. `BTreeMap` not `HashMap` so the iteration
/// order does not depend on host hashing state.
#[derive(Debug, Clone, Default)]
pub(crate) struct MmapperHandleTable {
    handles: BTreeMap<u32, MmapperHandle>,
}

impl MmapperHandleTable {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Record a fresh handle. The caller (332 or 362 dispatch) is
    /// responsible for the `mem_id` allocation via `Lv2Host::alloc_id`.
    pub(crate) fn insert(&mut self, mem_id: u32, handle: MmapperHandle) {
        let prior = self.handles.insert(mem_id, handle);
        debug_assert!(
            prior.is_none(),
            "mem_id {mem_id:#x} already in handle table; alloc_id collision",
        );
    }

    /// Look up a handle by `mem_id`. `None` matches RPCS3's CELL_ESRCH path
    /// at `sys_mmapper.cpp:662` when `idm::get` fails.
    pub(crate) fn get(&self, mem_id: u32) -> Option<MmapperHandle> {
        self.handles.get(&mem_id).copied()
    }
}

/// One pending region-install request emitted by `sys_mmapper_map_shared_memory`
/// (334) and drained by the runtime after `Lv2Host::dispatch` returns.
///
/// Region installation cannot ride the `Effect` pipeline today because
/// the commit path's pre-validation checks every write target against
/// the existing region table; a new region the same dispatch tried to
/// install would not yet exist at validation time. The runtime-side
/// drain pattern mirrors `drain_pending_invariant_breaks` and keeps
/// `Lv2Runtime` immutable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PendingRegionInstall {
    pub addr: u64,
    pub size: usize,
}
