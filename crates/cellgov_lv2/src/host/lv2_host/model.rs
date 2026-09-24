//! The `Lv2Host` and `FirmwareIdentity` types, construction, and the SDK, authority, permission and identity methods.

use std::collections::BTreeMap;

use cellgov_ps3_abi::format::elf::SYS_PROCESS_PARAM_SDK_VERSION_UNKNOWN;

use crate::fs_store::{FsMountTable, FsStore};
use crate::image::ContentStore;
use crate::ppu_thread::{PpuThreadTable, ThreadStackAllocator};
use crate::prx_registry::LoadedPrxRegistry;
use crate::sync_primitives::{
    CondTable, EventFlagTable, EventPortTable, EventQueueTable, LwMutexTable, MutexTable,
    SemaphoreTable,
};
use crate::thread_group::ThreadGroupTable;

use crate::host::mmapper::MmapperHandleTable;
use crate::host::process;
use crate::host::rsx::SysRsxContext;
use crate::host::{derived, observability, state};

/// LV2 host model driven by [`Self::dispatch`].
#[derive(Debug, Clone)]
pub struct Lv2Host {
    /// Hashed guest-visible state: every field folds into
    /// [`Self::state_hash`] by construction.
    pub(in crate::host) state: state::Lv2State,
    /// Unhashed guest-visible state; each field's doc names where a
    /// divergence in it is caught instead.
    pub(in crate::host) derived: derived::Lv2Derived,
    /// Instruments and diagnostics; inert with respect to
    /// guest-visible execution.
    pub(in crate::host) obs: observability::Lv2Observability,
}

/// Captured at boot via the verified `firmware.toml` manifest.
///
/// `image_version_hash` and `pup_sha256_bytes` together identify the
/// PUP the install came from; both fold into `Lv2Host::state_hash`.
#[derive(Debug, Clone)]
pub struct FirmwareIdentity {
    /// FNV-1a hash of the verified `image_version` string.
    pub image_version_hash: u64,
    /// Raw SHA-256 of the originating PUP file.
    pub pup_sha256_bytes: [u8; 32],
}

impl Default for Lv2Host {
    fn default() -> Self {
        Self::new()
    }
}

impl Lv2Host {
    /// Guest base of the 256 MB RSX-visible window.
    pub const SYS_RSX_MEM_BASE: u32 = 0x3000_0000;

    /// Upper bound (exclusive) of the sys_rsx memory region.
    pub const SYS_RSX_MEM_END: u32 = Self::SYS_RSX_MEM_BASE + 0x1000_0000;

    /// Lower bound (inclusive) of the `sys_mmapper_allocate_address`
    /// handout window. Set 256 MiB above `SYS_RSX_MEM_END` so the
    /// reserved `[0x4000_0000, 0x5000_0000)` rsx_context window
    /// (covering `sys_rsx::device::RSX_DEVICE_ADDR`) cannot alias an
    /// mmapper handout.
    pub const MMAPPER_REGION_START: u32 = 0x5000_0000;

    /// Upper bound (exclusive) of the `sys_mmapper_allocate_address`
    /// region. Capped below the RSX dma_control MMIO region at
    /// `control_register::DMA_CONTROL_BASE`.
    pub const MMAPPER_REGION_END: u32 = 0xC000_0000;

    /// Construct an empty host with default tables and id allocators.
    ///
    /// # Cross-module contract
    ///
    /// `/app_home/output.txt` also appears in
    /// `host::fs::FS_TTY_SINK_PATHS`; the open-flag validator
    /// exempts it from the EROFS branch. The two sites must agree;
    /// the `tty_sink_paths_are_pre_registered` regression in
    /// `host::fs::tests` pins this.
    pub fn new() -> Self {
        let mut fs_store = FsStore::new();
        fs_store
            .register_blob("/app_home/PARAM.SFO".to_string(), Vec::new())
            .expect("synthetic registration cannot collide on a fresh store");
        fs_store
            .register_blob("/app_home/output.txt".to_string(), Vec::new())
            .expect("synthetic registration cannot collide on a fresh store");
        Self {
            state: state::Lv2State {
                content: ContentStore::new(),
                groups: ThreadGroupTable::new(),
                ppu_threads: PpuThreadTable::new(),
                stack_allocator: ThreadStackAllocator::new(),
                next_kernel_id: crate::host::state::FIRST_KERNEL_ID,
                mem_alloc_ptr: 0x0001_0000, // PS3 user-memory region start
                mmapper_addr_cursor: Self::MMAPPER_REGION_START,
                rsx_mem_alloc_ptr: Self::SYS_RSX_MEM_BASE,
                rsx_mem_handle_counter: 1,
                rsx_context: SysRsxContext::new(),
                mmapper_handles: MmapperHandleTable::new(),
                mmapper_ipc: BTreeMap::new(),
                config: crate::host::config::ConfigTable::new(),
                uart: crate::host::uart::UartState::new(),
                usbd: crate::host::usbd::UsbdState::new(),
                memory_containers: std::collections::BTreeSet::new(),
                lwmutexes: LwMutexTable::new(),
                mutexes: MutexTable::new(),
                semaphores: SemaphoreTable::new(),
                event_queues: EventQueueTable::new(),
                event_ports: EventPortTable::new(),
                event_flags: EventFlagTable::new(),
                conds: CondTable::new(),
                lwmutex_holds: BTreeMap::new(),
                fs_store,
                prx_registry: LoadedPrxRegistry::new(),
                firmware_identity: None,
                processes: process::ProcessTable::new_boot(),
                process_counts: process::ProcessCounts::new(),
            },
            derived: derived::Lv2Derived {
                mem_alloc_base: 0x0001_0000,
                system_state_seeds: BTreeMap::new(),
                system_seeds_applied: std::collections::BTreeSet::new(),
                system_seed_bases: BTreeMap::new(),
                cond_ipc_keys: BTreeMap::new(),
                event_queue_ipc_keys: BTreeMap::new(),
                event_queue_ipc: BTreeMap::new(),
                pending_region_installs: Vec::new(),
                mmapper_install_ledger: BTreeMap::new(),
                fs_mounts: FsMountTable::new(),
                sdk_version: SYS_PROCESS_PARAM_SDK_VERSION_UNKNOWN,
                firmware_exports: BTreeMap::new(),
            },
            obs: observability::Lv2Observability::default(),
        }
    }

    /// Set the title's recorded SDK version (the value read from the
    /// title ELF's `process_param_t`).
    ///
    /// Callers that omit this leave the PS3 absent-case sentinel
    /// `0xFFFFFFFF` in place.
    pub fn set_sdk_version(&mut self, sdk_version: u32) {
        self.derived.sdk_version = sdk_version;
    }

    /// The value `sys_process_get_sdk_version` will write into the
    /// caller's `version_out_ptr`.
    #[inline]
    pub fn sdk_version(&self) -> u32 {
        self.derived.sdk_version
    }

    /// Set the boot process's program authority id (from the
    /// title SELF's identification header). Callers with raw-ELF
    /// input leave the retail-application fallback in place.
    pub fn set_program_authority_id(&mut self, authority_id: u64) {
        self.state.processes.boot_mut().authority_id = authority_id;
    }

    /// Boot process's program authority id.
    #[inline]
    pub fn program_authority_id(&self) -> u64 {
        self.state.processes.boot().authority_id
    }

    /// Per-process identity table.
    #[inline]
    pub fn processes(&self) -> &process::ProcessTable {
        &self.state.processes
    }

    /// Set `ctrl_flags1` from the booting SELF's plaintext capability
    /// header. Raw-ELF input and SELFs without the record leave 0.
    pub fn set_control_flags1(&mut self, flags: u32) {
        self.state.processes.boot_mut().control_flags1 = flags;
    }

    /// Install the firmware library -> NID -> OPD map the sc 484
    /// CoreOS branch resolves against.
    pub fn set_firmware_exports(
        &mut self,
        map: std::collections::BTreeMap<String, std::collections::BTreeMap<u32, u32>>,
    ) {
        self.derived.firmware_exports = map;
    }

    /// Install the unresolved-import NID -> requesting-libraries map
    /// the trampoline diagnostic names libraries from.
    pub fn set_unresolved_import_requesters(
        &mut self,
        map: std::collections::BTreeMap<u32, std::collections::BTreeSet<String>>,
    ) {
        self.obs.unresolved_import_requesters = map;
    }

    /// The host's instruments and diagnostics, read-only; the witness
    /// surface `BENCH_*` emitters and tests walk.
    pub fn observability(&self) -> &observability::Lv2Observability {
        &self.obs
    }

    /// Reset every instrument to its boot state.
    ///
    /// `pending_invariant_breaks` is carried over; the field's doc on
    /// [`observability::Lv2Observability`] names the contract.
    pub fn clear_observability(&mut self) {
        let pending = std::mem::take(&mut self.obs.pending_invariant_breaks);
        self.obs = observability::Lv2Observability::default();
        self.obs.pending_invariant_breaks = pending;
    }

    /// Raw capability word backing the privilege predicates.
    #[inline]
    pub fn control_flags1(&self) -> u32 {
        self.state.processes.boot().control_flags1
    }

    /// Whether the booting process holds root privilege.
    ///
    /// The three capability predicates share bits -- root implies
    /// debug-or-root, and the debug mask overlaps both.
    #[inline]
    pub fn has_root_perm(&self) -> bool {
        self.control_flags1() & cellgov_ps3_abi::format::sce::CTRL_FLAGS1_ROOT_MASK != 0
    }

    /// Whether the booting process holds debug or root privilege; the
    /// widest of the three masks. See [`Self::has_root_perm`].
    #[inline]
    pub fn debug_or_root(&self) -> bool {
        self.control_flags1() & cellgov_ps3_abi::format::sce::CTRL_FLAGS1_DEBUG_OR_ROOT_MASK != 0
    }

    /// Whether the booting process holds debug privilege. See
    /// [`Self::has_root_perm`].
    #[inline]
    pub fn has_debug_perm(&self) -> bool {
        self.control_flags1() & cellgov_ps3_abi::format::sce::CTRL_FLAGS1_DEBUG_MASK != 0
    }

    /// Whether the booting process is a CoreOS SELF (vsh and the other
    /// system executables).
    ///
    /// A CoreOS SELF is not necessarily root-capable: firmware
    /// libraries carry a CoreOS authority id with `ctrl_flags1 == 0`.
    #[inline]
    pub fn is_coreos(&self) -> bool {
        self.program_authority_id() >> 36
            == cellgov_ps3_abi::format::sce::COREOS_AUTHORITY_ID_PREFIX
    }

    /// Record the verified-firmware identity.
    ///
    /// # Panics
    /// Debug-only if an identity is already recorded; boot is
    /// one-shot.
    pub fn set_firmware_identity(&mut self, image_version: &str, pup_sha256_bytes: [u8; 32]) {
        debug_assert!(
            self.state.firmware_identity.is_none(),
            "firmware identity already set; boot is one-shot",
        );
        let mut h = cellgov_mem::Fnv1aHasher::new();
        h.write(image_version.as_bytes());
        self.state.firmware_identity = Some(FirmwareIdentity {
            image_version_hash: h.finish(),
            pup_sha256_bytes,
        });
    }

    /// `None` until boot records one.
    pub fn firmware_identity(&self) -> Option<&FirmwareIdentity> {
        self.state.firmware_identity.as_ref()
    }
}
