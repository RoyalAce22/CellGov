//! [`Lv2State`]: the hashed partition of [`super::Lv2Host`].

use std::collections::BTreeMap;

use crate::fs_store::FsStore;
use crate::image::ContentStore;
use crate::ppu_thread::{PpuThreadId, PpuThreadTable, ThreadStackAllocator};
use crate::prx_registry::LoadedPrxRegistry;
use crate::sync_primitives::{
    CondTable, EventFlagTable, EventPortTable, EventQueueTable, LwMutexTable, MutexTable,
    SemaphoreTable,
};
use crate::thread_group::ThreadGroupTable;

use super::lv2_host::FirmwareIdentity;
use super::mmapper::MmapperHandleTable;
use super::process;
use super::rsx::SysRsxContext;

/// Guest-visible LV2 state; every field folds into the host state
/// hash per [`Self::state_hash`]'s exhaustive-destructure contract.
#[derive(Debug, Clone)]
pub(in crate::host) struct Lv2State {
    pub(in crate::host) content: ContentStore,
    pub(in crate::host) groups: ThreadGroupTable,
    pub(in crate::host) ppu_threads: PpuThreadTable,
    pub(in crate::host) stack_allocator: ThreadStackAllocator,
    /// Shared id allocator for mutex / semaphore / event-queue /
    /// event-flag / cond. `lwmutexes` has its own allocator from 1.
    pub(in crate::host) next_kernel_id: u32,
    pub(in crate::host) mem_alloc_ptr: u32,
    /// Bump cursor for `sys_mmapper_allocate_address` (256 MiB+ chunks).
    pub(in crate::host) mmapper_addr_cursor: u32,
    pub(in crate::host) rsx_mem_alloc_ptr: u32,
    pub(in crate::host) rsx_mem_handle_counter: u32,
    pub(in crate::host) rsx_context: SysRsxContext,
    /// Populated by 332 / 362, consumed by 334 / 337.
    pub(in crate::host) mmapper_handles: MmapperHandleTable,
    /// `ipc_key -> mem_id` for process-shared mmapper allocations.
    /// A keyed 332 with a registered key returns the existing
    /// `mem_id` (RPCS3's SYS_SYNC_NOT_CARE path,
    /// `sys_mmapper.cpp` `create_lv2_shm`); an unregistered key mints and
    /// registers. The association steers a future 332's answer and is
    /// recorded nowhere else.
    pub(in crate::host) mmapper_ipc: BTreeMap<u64, u32>,
    pub(in crate::host) lwmutexes: LwMutexTable,
    pub(in crate::host) mutexes: MutexTable,
    pub(in crate::host) semaphores: SemaphoreTable,
    pub(in crate::host) event_queues: EventQueueTable,
    pub(in crate::host) event_ports: EventPortTable,
    pub(in crate::host) event_flags: EventFlagTable,
    pub(in crate::host) conds: CondTable,
    /// Per-thread count of distinct lwmutexes held. Recursive
    /// re-acquires of the same lwmutex do not bump the count; only
    /// first-acquire (FREE -> me) and kernel-side transfer
    /// (LwMutexWake) do.
    pub(in crate::host) lwmutex_holds: BTreeMap<PpuThreadId, u32>,
    pub(in crate::host) fs_store: FsStore,
    /// Firmware modules loaded at boot. Empty when no firmware-dir
    /// was configured. Guest-mutable (sc 480 mints miss stubs).
    pub(in crate::host) prx_registry: LoadedPrxRegistry,
    /// Folded when set.
    pub(in crate::host) firmware_identity: Option<FirmwareIdentity>,
    /// Per-process identity ([`process::ProcessEntry`]). Boot seeds
    /// the boot entry from the title SELF's plaintext headers;
    /// raw-ELF inputs keep the retail-application fallback.
    pub(in crate::host) processes: process::ProcessTable,
    /// Feeds `sys_process_get_number_of_object`. Folded when
    /// non-zero.
    pub(in crate::host) process_counts: process::ProcessCounts,
}
