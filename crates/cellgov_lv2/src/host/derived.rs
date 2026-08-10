//! [`Lv2Derived`]: the unhashed-but-guest-visible partition of
//! [`super::Lv2Host`].
//!
//! The bucket invariant is not "not hashed" -- it is "unhashed
//! because a divergence here is caught elsewhere." Every field's doc
//! names where. A field with no such answer belongs in
//! [`super::state::Lv2State`].

use std::collections::{BTreeMap, BTreeSet};

use crate::fs_store::FsMountTable;

use super::mmapper::{PendingRegionInstall, SystemStateSeed};

/// Guest-visible state excluded from the host state hash, each field
/// with the reason a divergence in it is caught elsewhere.
#[derive(Debug, Clone)]
pub(in crate::host) struct Lv2Derived {
    /// Where `mem_alloc_ptr` started this boot; `ptr - base` is the
    /// consumed-bytes figure `sys_memory_get_user_memory_size`
    /// subtracts from the reported total. Boot config;
    /// `mem_alloc_ptr` already carries the allocator into the hash.
    pub(in crate::host) mem_alloc_base: u32,
    /// Boot-registered seeds keyed by `shm_ipc_key`; immutable after
    /// boot. Applied at most once, on the first 334 / 337 map of the
    /// matching shm. The seed writes settle via `GuestMemory` (hashed
    /// there).
    pub(in crate::host) system_state_seeds: BTreeMap<u64, SystemStateSeed>,
    /// Keys whose seed has been applied; suppresses re-application on
    /// a re-map. Derived from the seed map and the hashed seed
    /// writes, which settle via `GuestMemory`.
    pub(in crate::host) system_seeds_applied: BTreeSet<u64>,
    /// `shm_ipc_key -> mapped guest base` recorded when a seed is
    /// applied. The cond ring-check wake reads slot state through
    /// this base. Derived from the hashed map effects.
    pub(in crate::host) system_seed_bases: BTreeMap<u64, u32>,
    /// `cond id -> create-time ipc_key` for process-shared conds
    /// (key 0 entries are not stored). Create-time config mirrored
    /// from guest memory (hashed there).
    pub(in crate::host) cond_ipc_keys: BTreeMap<u32, u64>,
    /// `event-queue id -> create-time ipc_key` for keyed queues (key 0
    /// entries are not stored), mirroring [`Self::cond_ipc_keys`].
    /// Create-time config; a divergent binding surfaces in the hashed
    /// port table when a connect resolves through it.
    pub(in crate::host) event_queue_ipc_keys: BTreeMap<u32, u64>,
    /// `ipc_key -> queue id` for keyed event queues, in creation
    /// order. A divergent entry surfaces in the hashed port table
    /// when `sys_event_port_connect_ipc` resolves through it.
    pub(in crate::host) event_queue_ipc: BTreeMap<u64, u32>,
    /// Pending region-install requests emitted by 334 / 337. Drained
    /// by the runtime post-dispatch and applied to `GuestMemory`
    /// (hashed there) before the dispatch's effects commit.
    pub(in crate::host) pending_region_installs: Vec<PendingRegionInstall>,
    /// Ledger of every mmapper-window range handed out via 334 / 337
    /// and not yet released. The search in 337 consults this to find a
    /// free aligned range at-or-after the caller's hint; keyed
    /// `start_addr -> size` so the nearest-below lookup is `O(log n)`.
    /// Every insert pairs in the same dispatch with a
    /// `PendingRegionInstall` and the chosen-address write, both
    /// settling in hashed `GuestMemory`, so a ledger divergence is
    /// caught at the step it arises.
    pub(in crate::host) mmapper_install_ledger: BTreeMap<u32, u32>,
    /// Consulted by the dispatch layer when a guest path is not
    /// pre-registered in the fs store. Boot populates from the
    /// title manifest; immutable thereafter.
    pub(in crate::host) fs_mounts: FsMountTable,
    /// `sys_process_get_sdk_version` return value. Read from the
    /// title ELF's `sys_proc_param` segment at boot (see
    /// `cellgov_ppu::loader::find_sys_process_param`). RPCS3 mirror:
    /// `g_ps3_process_info.sdk_ver` set from the LOOS+1 program
    /// header's `process_param_t.sdk_version`
    /// (rpcs3 PPUModule.cpp). The PS3 sentinel for
    /// absent param segment is `0xFFFFFFFF` (the same one PSL1GHT
    /// homebrew sees); this default matches that contract for
    /// callers that never invoke `set_sdk_version`. Boot-time
    /// constant read from the title image, whose bytes are hashed in
    /// `GuestMemory`.
    pub(in crate::host) sdk_version: u32,
    /// Firmware library name -> NID -> OPD address, supplied at boot
    /// from the loaded firmware set. The sc 484 CoreOS branch resolves
    /// a caller's import NIDs against it under the library each import
    /// entry names; empty for a boot with no firmware set, which makes
    /// every import unresolved rather than wrong. Immutable
    /// boot-derived data: the firmware identity it derives from folds
    /// into the hash, and the GOT writes it steers settle via
    /// `GuestMemory`.
    pub(in crate::host) firmware_exports: BTreeMap<String, BTreeMap<u32, u32>>,
}
