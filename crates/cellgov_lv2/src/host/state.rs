//! [`Lv2State`]: the hashed partition of [`super::Lv2Host`].

use crate::fs_store::FsStore;
use crate::image::ContentStore;
use crate::ppu_thread::{PpuThreadId, PpuThreadTable, ThreadStackAllocator};
use crate::prx_registry::LoadedPrxRegistry;
use crate::raw_spu::RawSpuTable;
use crate::sync_primitives::{
    CondTable, EventFlagTable, EventPortTable, EventQueueTable, LwMutexTable, MutexTable,
    SemaphoreTable,
};
use crate::thread_group::ThreadGroupTable;

use super::config::ConfigTable;
use super::kernel_ids::KernelIdTable;
use super::lv2_host::FirmwareIdentity;
use super::mmapper::MmapperHandleTable;
use super::process;
use super::rsx::SysRsxContext;
use super::uart::UartState;
use super::usbd::UsbdState;

/// Guest-visible LV2 state; every field enters the host's sync partial
/// per the exhaustive destructure in `Lv2State::sync_partial`.
#[derive(Debug, Clone)]
pub(in crate::host) struct Lv2State {
    pub(in crate::host) content: ContentStore,
    pub(in crate::host) groups: ThreadGroupTable,
    /// Raw SPUs (150-154, 160, 161, 163, 196, 197, 199); `groups` keeps
    /// the limit `sys_spu_initialize` sets for them.
    pub(in crate::host) raw_spus: RawSpuTable,
    pub(in crate::host) ppu_threads: PpuThreadTable,
    pub(in crate::host) stack_allocator: ThreadStackAllocator,
    /// The slot table every class-tagged kernel id comes from.
    pub(in crate::host) kernel_ids: KernelIdTable,
    /// The serial the next memory container id carries.
    pub(in crate::host) next_container_serial: u32,
    pub(in crate::host) mem_alloc_ptr: u32,
    pub(in crate::host) rsx_mem_alloc_ptr: u32,
    pub(in crate::host) rsx_mem_handle_counter: u32,
    pub(in crate::host) rsx_context: SysRsxContext,
    /// Populated by 332 / 362, consumed by 334 / 337.
    pub(in crate::host) mmapper_handles: MmapperHandleTable,
    /// `ipc_key -> mem_id` for process-shared mmapper allocations.
    /// A keyed 332 with a registered key returns the existing
    /// `mem_id`; an unregistered key mints and registers. The
    /// association steers a future 332's answer and is recorded
    /// nowhere else.
    pub(in crate::host) mmapper_ipc: cellgov_mem::lanes::LaneMap<u64, u32>,
    /// The kernel SPU image ids 156 and 157 handed out, each to the
    /// content-store handle of its image, or to 0 once 158 closed it.
    /// A kernel record naming a closed id answers CELL_ESRCH.
    pub(in crate::host) spu_image_ids: cellgov_mem::lanes::LaneMap<u32, u32>,
    /// The address areas 330 reserved and no 331 freed, by base: what
    /// 334 and 337 place a segment inside.
    pub(in crate::host) mmapper_areas:
        cellgov_mem::lanes::LaneMap<u32, super::mmapper::MmapperArea>,
    /// `sys_config` handles, services, listeners, and the events a
    /// listener may still read back (516-522).
    pub(in crate::host) config: ConfigTable,
    /// The AV manager's virtual UART: reply stream, parked readers, and
    /// HDMI state (367-370).
    pub(in crate::host) uart: UartState,
    /// The USB host driver: handles and parked event readers
    /// (530-541).
    pub(in crate::host) usbd: UsbdState,
    /// The containers `sys_memory_container_create` minted, by id: the
    /// membership `sys_memory_allocate_from_container` checks, and the
    /// memory SPU thread groups and allocations take from each.
    pub(in crate::host) memory_containers:
        cellgov_mem::lanes::LaneMap<u32, super::memory::MemoryContainer>,
    /// The live blocks `sys_memory_allocate` and
    /// `sys_memory_allocate_from_container` handed out, by base address:
    /// what `sys_memory_free` returns and what the available user
    /// memory leaves out.
    pub(in crate::host) memory_blocks: cellgov_mem::lanes::LaneMap<u32, super::memory::MemoryBlock>,
    pub(in crate::host) lwmutexes: LwMutexTable,
    pub(in crate::host) mutexes: MutexTable,
    pub(in crate::host) semaphores: SemaphoreTable,
    pub(in crate::host) event_queues: EventQueueTable,
    pub(in crate::host) event_ports: EventPortTable,
    pub(in crate::host) event_flags: EventFlagTable,
    pub(in crate::host) conds: CondTable,
    pub(in crate::host) lwconds: crate::sync_primitives::LwCondTable,
    /// Per-thread count of distinct lwmutexes held. Recursive
    /// re-acquires of the same lwmutex do not bump the count; only
    /// first-acquire (FREE -> me) and kernel-side transfer
    /// (LwMutexWake) do.
    pub(in crate::host) lwmutex_holds: cellgov_mem::lanes::LaneMap<PpuThreadId, u32>,
    pub(in crate::host) fs_store: FsStore,
    /// Firmware modules loaded at boot. Empty when no firmware-dir
    /// was configured. Guest-mutable (sc 480 mints miss stubs).
    pub(in crate::host) prx_registry: LoadedPrxRegistry,
    pub(in crate::host) firmware_identity: Option<FirmwareIdentity>,
    /// Per-process identity ([`process::ProcessEntry`]). Boot seeds
    /// the boot entry from the title SELF's plaintext headers;
    /// raw-ELF inputs keep the retail-application fallback.
    pub(in crate::host) processes: process::ProcessTable,
    /// Feeds `sys_process_get_number_of_object`.
    pub(in crate::host) process_counts: process::ProcessCounts,
}
