//! `Lv2Host` model, its `FirmwareIdentity` payload, and the state
//! primitives exposed to the dispatch submodules.

use std::collections::BTreeMap;

use cellgov_event::UnitId;
use cellgov_ps3_abi::elf::SYS_PROCESS_PARAM_SDK_VERSION_UNKNOWN;

use crate::fs_store::{FsMountTable, FsStore};
use crate::image::ContentStore;
use crate::ppu_thread::{
    PpuThread, PpuThreadAttrs, PpuThreadId, PpuThreadTable, ThreadStack, ThreadStackAllocator,
    TlsTemplate,
};
use crate::prx_registry::LoadedPrxRegistry;
use crate::sync_primitives::{
    CondTable, EventFlagTable, EventPortTable, EventQueueTable, LwMutexTable, MutexTable,
    SemaphoreTable,
};
use crate::thread_group::ThreadGroupTable;

use super::mmapper::{MmapperHandleTable, SystemStateSeed};
use super::process;
use super::rsx::SysRsxContext;
use super::system_ipc_witness::SystemIpcMapping;
use super::{derived, observability, state};

/// LV2 host model driven by [`Self::dispatch`].
#[derive(Debug, Clone)]
pub struct Lv2Host {
    /// Hashed guest-visible state: every field folds into
    /// [`Self::state_hash`] by construction.
    pub(super) state: state::Lv2State,
    /// Unhashed guest-visible state; each field's doc names where a
    /// divergence in it is caught instead.
    pub(super) derived: derived::Lv2Derived,
    /// Instruments and diagnostics; inert with respect to
    /// guest-visible execution.
    pub(super) obs: observability::Lv2Observability,
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
                tls_template: TlsTemplate::empty(),
                stack_allocator: ThreadStackAllocator::new(),
                next_kernel_id: 0x4000_0001, // non-zero to catch uninitialized use
                mem_alloc_ptr: 0x0001_0000,  // PS3 user-memory region start
                mmapper_addr_cursor: Self::MMAPPER_REGION_START,
                rsx_mem_alloc_ptr: Self::SYS_RSX_MEM_BASE,
                rsx_mem_handle_counter: 1,
                rsx_context: SysRsxContext::new(),
                mmapper_handles: MmapperHandleTable::new(),
                mmapper_ipc: BTreeMap::new(),
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
                program_authority_id: cellgov_ps3_abi::sce::RETAIL_APP_PROGRAM_AUTHORITY_ID,
                control_flags1: 0,
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
            // Derived `Default` IS the boot state, so
            // `clear_observability` cannot drift from `new`.
            obs: observability::Lv2Observability::default(),
        }
    }

    /// Set the title's recorded SDK version (the value read from the
    /// title ELF's `process_param_t`). Boot reads it via
    /// `cellgov_ppu::loader::find_sys_process_param` and plumbs the
    /// parsed `sdk_version` through. Callers that omit this leave the
    /// PS3 absent-case sentinel `0xFFFFFFFF` in place.
    pub fn set_sdk_version(&mut self, sdk_version: u32) {
        self.derived.sdk_version = sdk_version;
    }

    /// The value `sys_process_get_sdk_version` will write into the
    /// caller's `version_out_ptr`.
    #[inline]
    pub fn sdk_version(&self) -> u32 {
        self.derived.sdk_version
    }

    /// Set the booting process's program authority id (from the
    /// title SELF's identification header). Callers with raw-ELF
    /// input leave the retail-application fallback in place.
    pub fn set_program_authority_id(&mut self, authority_id: u64) {
        self.state.program_authority_id = authority_id;
    }

    /// The value `sys_ss_access_control_engine` pkg 2 serves.
    #[inline]
    pub fn program_authority_id(&self) -> u64 {
        self.state.program_authority_id
    }

    /// Set `ctrl_flags1` from the booting SELF's plaintext capability
    /// header. Raw-ELF input and SELFs without the record leave 0.
    pub fn set_control_flags1(&mut self, flags: u32) {
        self.state.control_flags1 = flags;
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
    /// Test hook for the observability inertness gate: a boot that
    /// clears this after every committed step must produce the same
    /// state-hash sequence and trace bytes as one that records.
    ///
    /// `pending_invariant_breaks` is carried over, not reset; the
    /// field's doc on [`observability::Lv2Observability`] names the
    /// contract.
    pub fn clear_observability(&mut self) {
        let pending = std::mem::take(&mut self.obs.pending_invariant_breaks);
        self.obs = observability::Lv2Observability::default();
        self.obs.pending_invariant_breaks = pending;
    }

    /// Raw capability word backing the privilege predicates.
    #[inline]
    pub fn control_flags1(&self) -> u32 {
        self.state.control_flags1
    }

    /// Whether the booting process holds root privilege.
    ///
    /// The three capability predicates share bits -- root implies
    /// debug-or-root, and the debug mask overlaps both. They are
    /// mirrored from the oracle rather than reduced to disjoint bits,
    /// because the exact per-bit meaning is unconfirmed there too.
    #[inline]
    pub fn has_root_perm(&self) -> bool {
        self.state.control_flags1 & cellgov_ps3_abi::sce::CTRL_FLAGS1_ROOT_MASK != 0
    }

    /// Whether the booting process holds debug or root privilege; the
    /// widest of the three masks. See [`Self::has_root_perm`].
    #[inline]
    pub fn debug_or_root(&self) -> bool {
        self.state.control_flags1 & cellgov_ps3_abi::sce::CTRL_FLAGS1_DEBUG_OR_ROOT_MASK != 0
    }

    /// Whether the booting process holds debug privilege. See
    /// [`Self::has_root_perm`].
    #[inline]
    pub fn has_debug_perm(&self) -> bool {
        self.state.control_flags1 & cellgov_ps3_abi::sce::CTRL_FLAGS1_DEBUG_MASK != 0
    }

    /// Whether the booting process is a CoreOS SELF (vsh and the other
    /// system executables). Derived from the program authority id, not
    /// from `ctrl_flags1`: the two are independent, and a CoreOS SELF
    /// is not necessarily root-capable (firmware libraries carry a
    /// CoreOS authority id with `ctrl_flags1 == 0`).
    #[inline]
    pub fn is_coreos(&self) -> bool {
        self.state.program_authority_id >> 36 == cellgov_ps3_abi::sce::COREOS_AUTHORITY_ID_PREFIX
    }

    /// Record the verified-firmware identity. Boot is one-shot; a
    /// second call panics in debug builds.
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

    /// In-memory filesystem store.
    pub fn fs_store(&self) -> &FsStore {
        &self.state.fs_store
    }

    /// Mutable view of [`Self::fs_store`].
    pub fn fs_store_mut(&mut self) -> &mut FsStore {
        &mut self.state.fs_store
    }

    /// Guest-path to host-path mount table.
    pub fn fs_mounts(&self) -> &FsMountTable {
        &self.derived.fs_mounts
    }

    /// Mutable view; written by boot only.
    pub fn fs_mounts_mut(&mut self) -> &mut FsMountTable {
        &mut self.derived.fs_mounts
    }

    /// Distinct lwmutexes currently held by `tid`.
    pub fn lwmutex_holds_for(&self, tid: PpuThreadId) -> u32 {
        self.state.lwmutex_holds.get(&tid).copied().unwrap_or(0)
    }

    /// Bumps the count for a first-acquire (FREE -> tid) or a
    /// kernel-side transfer. Recursive re-acquires (tid already
    /// the owner) are tracked elsewhere and must not pass through
    /// this entry.
    pub fn lwmutex_holds_inc(&mut self, tid: PpuThreadId) {
        let slot = self.state.lwmutex_holds.entry(tid).or_insert(0);
        debug_assert!(*slot < u32::MAX, "lwmutex hold count overflow on {tid:?}",);
        *slot += 1;
    }

    /// Release builds saturate at 0 so a leak does not corrupt
    /// downstream counters.
    pub fn lwmutex_holds_dec(&mut self, tid: PpuThreadId) {
        if let Some(slot) = self.state.lwmutex_holds.get_mut(&tid) {
            debug_assert!(*slot > 0, "lwmutex hold count underflow on {tid:?}",);
            *slot = slot.saturating_sub(1);
            if *slot == 0 {
                self.state.lwmutex_holds.remove(&tid);
            }
        } else {
            debug_assert!(
                false,
                "lwmutex_holds_dec on {tid:?} with no entry; inc/dec pairing leaked",
            );
        }
    }

    /// Used at thread-exit and stale-owner recovery so a dead
    /// thread's count does not leak.
    pub fn lwmutex_holds_clear(&mut self, tid: PpuThreadId) {
        self.state.lwmutex_holds.remove(&tid);
    }

    /// `false` when `unit` has no PPU thread mapping.
    pub fn unit_holds_lwmutex(&self, unit: UnitId) -> bool {
        match self.state.ppu_threads.thread_id_for_unit(unit) {
            Some(tid) => self.lwmutex_holds_for(tid) > 0,
            None => false,
        }
    }

    /// See [`process::ProcessCounts::fs_fd_inc`] for the no-decrement
    /// contract.
    pub(super) fn fs_fd_count_inc(&mut self) {
        self.state.process_counts.fs_fd_inc();
    }

    /// Increment the live `sys_lwcond` object count.
    pub fn lwcond_count_inc(&mut self) {
        self.state.process_counts.lwcond_inc();
    }

    /// Decrement the live `sys_lwcond` count; saturates at 0.
    pub fn lwcond_count_dec(&mut self) {
        self.state.process_counts.lwcond_dec();
    }

    /// Callers that load a real ELF must set this to the
    /// 64KB-aligned address above the ELF's highest PT_LOAD end;
    /// the default (`0x0001_0000`) overwrites the image otherwise.
    pub fn set_mem_alloc_base(&mut self, base: u32) {
        debug_assert!(
            base & 0xFFFF == 0,
            "mem_alloc_base must be 64 KiB aligned, got {base:#x}",
        );
        debug_assert!(
            base >= 0x0001_0000,
            "mem_alloc_base must sit at or above the PS3 user-memory floor (0x0001_0000), got {base:#x}",
        );
        debug_assert!(
            base < Self::SYS_RSX_MEM_BASE,
            "mem_alloc_base must sit below SYS_RSX_MEM_BASE ({:#x}), got {base:#x}",
            Self::SYS_RSX_MEM_BASE,
        );
        self.state.mem_alloc_ptr = base;
        self.derived.mem_alloc_base = base;
    }

    /// sys_rsx host context.
    #[inline]
    pub fn sys_rsx_context(&self) -> &SysRsxContext {
        &self.state.rsx_context
    }

    /// Record an iomap mapping without going through 672. Synthetic
    /// test scenarios use this to wire up the IO -> EA translation
    /// the FIFO advance pass needs without booting the firmware-set
    /// `sys_rsx_context_iomap` path. Production code calls
    /// `dispatch_sys_rsx_context_iomap`, which validates against the
    /// 672 contract.
    pub fn seed_rsx_iomap(&mut self, io: u32, ea: u32, size: u32) {
        self.state.rsx_context.iomap_io = io;
        self.state.rsx_context.iomap_ea = ea;
        self.state.rsx_context.iomap_size = size;
    }

    /// Mark the sys_rsx context as allocated under `context_id`
    /// without going through 670. Synthetic test scenarios use this
    /// to satisfy the `allocated && matching id` guard at the top of
    /// `sys_rsx_context_attribute` (674) without the OUT-pointer
    /// memory plumbing 670 requires. Production code calls
    /// `dispatch_sys_rsx_context_allocate`.
    pub fn seed_rsx_context_allocated(&mut self, context_id: u32) {
        self.state.rsx_context.allocated = true;
        self.state.rsx_context.context_id = context_id;
    }

    pub(super) fn alloc_id(&mut self) -> u32 {
        let id = self.state.next_kernel_id;
        self.state.next_kernel_id = self
            .state
            .next_kernel_id
            .checked_add(1)
            .expect("kernel id space exhausted");
        id
    }

    /// Bump the mmapper VM cursor by `size` rounded up to the
    /// 256 MiB granule and return the pre-bump cursor.
    ///
    /// Returns `None` for `size == 0`, when the bump would overflow
    /// `u32`, or when the resulting range would cross
    /// [`Self::MMAPPER_REGION_END`].
    pub(super) fn mmapper_alloc(&mut self, size: u32) -> Option<u32> {
        if size == 0 {
            return None;
        }
        let granule = 0x1000_0000u32;
        let rounded = size.checked_add(granule - 1)? & !(granule - 1);
        let base = self.state.mmapper_addr_cursor;
        let next = base.checked_add(rounded)?;
        if next > Self::MMAPPER_REGION_END {
            return None;
        }
        self.state.mmapper_addr_cursor = next;
        Some(base)
    }

    /// Search for the first free, `align`-aligned range of `size`
    /// bytes inside `[MMAPPER_REGION_START, MMAPPER_REGION_END)` at
    /// or after `hint`, skipping over every range currently recorded
    /// in [`Self::mmapper_install_ledger`].
    ///
    /// `hint` is rounded UP to `align`; misaligned hints do not
    /// fail. RPCS3's `area->alloc` does the same (the
    /// `start_addr != area->addr` check at
    /// `tools/rpcs3-src/rpcs3/Emu/Cell/lv2/sys_mmapper.cpp`
    /// `sys_mmapper_search_and_map` is
    /// area selection, not in-area alignment).
    ///
    /// Returns `None` on exhaustion (matches RPCS3's `CELL_ENOMEM`
    /// path in `sys_mmapper_search_and_map`).
    pub(super) fn mmapper_search_free_range(
        &self,
        hint: u32,
        size: u32,
        align: u32,
    ) -> Option<u32> {
        debug_assert!(
            align.is_power_of_two(),
            "mmapper align must be a power of two"
        );
        debug_assert!(align != 0, "mmapper align must be non-zero");
        if size == 0 {
            return None;
        }
        let align_mask = align - 1;
        let hint_clamped = hint.max(Self::MMAPPER_REGION_START);
        let mut candidate = hint_clamped.checked_add(align_mask)? & !align_mask;
        loop {
            let end = candidate.checked_add(size)?;
            if end > Self::MMAPPER_REGION_END {
                return None;
            }
            // Find the closest ledger entry whose start is < end. If
            // its [start, start+len) overlaps [candidate, end), advance.
            let prior = self
                .derived
                .mmapper_install_ledger
                .range(..end)
                .next_back()
                .map(|(&start, &len)| (start, len));
            match prior {
                Some((start, len)) => {
                    let prior_end = start.checked_add(len)?;
                    if prior_end > candidate {
                        // Overlap: advance past prior_end, re-align.
                        candidate = prior_end.checked_add(align_mask)? & !align_mask;
                        continue;
                    }
                    return Some(candidate);
                }
                None => return Some(candidate),
            }
        }
    }

    /// Record an mmapper-window install in the host ledger. Paired
    /// with a `PendingRegionInstall` push by the same dispatch.
    pub(super) fn mmapper_ledger_insert(&mut self, addr: u32, size: u32) {
        let prior = self.derived.mmapper_install_ledger.insert(addr, size);
        debug_assert!(
            prior.is_none(),
            "mmapper ledger: addr {addr:#x} already recorded (size {prior:?})",
        );
    }

    /// `ipc_key -> mem_id` registrations made by keyed 332 calls.
    pub fn mmapper_ipc(&self) -> &BTreeMap<u64, u32> {
        &self.state.mmapper_ipc
    }

    /// Register a boot-state seed; a duplicate `shm_ipc_key` replaces
    /// the prior entry (last-write-wins). Boot-only: registering
    /// after the matching shm has been mapped has no effect.
    pub fn register_system_seed(&mut self, seed: SystemStateSeed) {
        self.derived
            .system_state_seeds
            .insert(seed.shm_ipc_key, seed);
    }

    /// Boot-registered seeds keyed by `shm_ipc_key`.
    pub fn system_state_seeds(&self) -> &BTreeMap<u64, SystemStateSeed> {
        &self.derived.system_state_seeds
    }

    /// `true` once the seed registered under `shm_ipc_key` has been
    /// applied by a 334 / 337 map.
    pub fn system_seed_applied(&self, shm_ipc_key: u64) -> bool {
        self.derived.system_seeds_applied.contains(&shm_ipc_key)
    }

    /// Mapped guest base of the seeded shm, once applied.
    pub fn system_seed_base(&self, shm_ipc_key: u64) -> Option<u32> {
        self.derived.system_seed_bases.get(&shm_ipc_key).copied()
    }

    /// Count of event queues registered under an ipc key.
    pub fn keyed_event_queue_count(&self) -> usize {
        self.derived.event_queue_ipc.len()
    }

    /// Count committed writes that land in a namespace-keyed shm.
    ///
    /// # Cross-module contract
    ///
    /// The runtime must call this once per successful commit, with the
    /// same effect slice the commit pipeline applied. Calling it before
    /// the commit succeeds would count writes a fault discarded.
    ///
    /// O(writes * mappings), and returns on the first line for a boot
    /// that mapped no namespace shm at all.
    pub fn note_committed_effects(&mut self, effects: &[cellgov_effects::Effect]) {
        if self.obs.system_ipc_mappings.is_empty() {
            return;
        }
        for effect in effects {
            let cellgov_effects::Effect::SharedWriteIntent { range, .. } = effect else {
                continue;
            };
            let start = range.start().raw();
            let end = start.saturating_add(range.length());
            let hit = self
                .obs
                .system_ipc_mappings
                .values()
                .find(|m| {
                    let m_start = u64::from(m.base);
                    start < m_start + u64::from(m.size) && m_start < end
                })
                .map(|m| m.ipc_key);
            if let Some(ipc_key) = hit {
                self.obs.system_ipc_witness.shm_writes += 1;
                self.obs.system_ipc_witness.note_key(ipc_key);
            }
        }
    }

    /// Record a namespace-keyed shm mapping and bump the map witness.
    pub(super) fn note_system_ipc_map(&mut self, mem_id: u32, base: u32, size: u32) {
        let Some((&ipc_key, _)) = self.state.mmapper_ipc.iter().find(|&(_, &id)| id == mem_id)
        else {
            return;
        };
        if !super::is_system_ipc_key(ipc_key) {
            return;
        }
        self.obs.system_ipc_mappings.insert(
            base,
            SystemIpcMapping {
                ipc_key,
                base,
                size,
            },
        );
        self.obs.system_ipc_witness.shm_maps += 1;
        self.obs.system_ipc_witness.note_key(ipc_key);
    }

    /// Read-only `pending_region_installs` snapshot used by sibling
    /// dispatch-arm tests. Not a drain; the runtime is still the
    /// authoritative drain consumer.
    #[cfg(all(test, debug_assertions))]
    pub(super) fn drain_pending_region_installs_inspect(
        &self,
    ) -> &[super::mmapper::PendingRegionInstall] {
        &self.derived.pending_region_installs
    }

    /// Per-title content manifest store.
    pub fn content_store(&self) -> &ContentStore {
        &self.state.content
    }

    /// Mutable view of [`Self::content_store`].
    pub fn content_store_mut(&mut self) -> &mut ContentStore {
        &mut self.state.content
    }

    /// Loaded-PRX registry.
    pub fn prx_registry(&self) -> &LoadedPrxRegistry {
        &self.state.prx_registry
    }

    /// Mutable view of [`Self::prx_registry`].
    pub fn prx_registry_mut(&mut self) -> &mut LoadedPrxRegistry {
        &mut self.state.prx_registry
    }

    /// SPU thread-group table.
    pub fn thread_groups(&self) -> &ThreadGroupTable {
        &self.state.groups
    }

    /// Mutable view of [`Self::thread_groups`].
    pub fn thread_groups_mut(&mut self) -> &mut ThreadGroupTable {
        &mut self.state.groups
    }

    /// PPU thread table.
    pub fn ppu_threads(&self) -> &PpuThreadTable {
        &self.state.ppu_threads
    }

    /// Mutable view of [`Self::ppu_threads`].
    pub fn ppu_threads_mut(&mut self) -> &mut PpuThreadTable {
        &mut self.state.ppu_threads
    }

    /// Call exactly once after the primary PPU unit is registered.
    pub fn seed_primary_ppu_thread(&mut self, unit_id: UnitId, attrs: PpuThreadAttrs) {
        self.state.ppu_threads.insert_primary(unit_id, attrs);
    }

    /// Alias a transient unit (e.g. a per-module module_start unit)
    /// to the primary thread so sync-syscall dispatch resolves the
    /// caller. Mirrors real LV2's "module_start runs on the calling
    /// thread" contract. See
    /// [`PpuThreadTable::alias_unit`][crate::ppu_thread::PpuThreadTable::alias_unit].
    pub fn alias_unit_to_primary(&mut self, unit_id: UnitId) -> bool {
        self.state
            .ppu_threads
            .alias_unit(unit_id, PpuThreadId::PRIMARY)
    }

    /// Drop an alias previously installed via [`Self::alias_unit_to_primary`].
    pub fn drop_ppu_thread_alias(&mut self, unit_id: UnitId) -> bool {
        self.state.ppu_threads.drop_alias(unit_id)
    }

    /// PPU thread record bound to `unit_id`, if any.
    pub fn ppu_thread_for_unit(&self, unit_id: UnitId) -> Option<&PpuThread> {
        self.state.ppu_threads.get_by_unit(unit_id)
    }

    /// PPU thread id bound to `unit_id`, if any.
    pub fn ppu_thread_id_for_unit(&self, unit_id: UnitId) -> Option<PpuThreadId> {
        self.state.ppu_threads.thread_id_for_unit(unit_id)
    }

    /// `false` when `unit_id` has no PPU mapping.
    pub fn is_ppu_thread_finished_for_unit(&self, unit_id: UnitId) -> bool {
        match self.state.ppu_threads.get_by_unit(unit_id) {
            Some(thread) => thread.state.is_finished(),
            None => false,
        }
    }

    /// Install the TLS template used for new PPU threads.
    pub fn set_tls_template(&mut self, template: TlsTemplate) {
        self.state.tls_template = template;
    }

    /// Installed TLS template.
    pub fn tls_template(&self) -> &TlsTemplate {
        &self.state.tls_template
    }

    /// Lightweight mutex table.
    pub fn lwmutexes(&self) -> &LwMutexTable {
        &self.state.lwmutexes
    }

    /// Mutable view of [`Self::lwmutexes`].
    pub fn lwmutexes_mut(&mut self) -> &mut LwMutexTable {
        &mut self.state.lwmutexes
    }

    /// Mutex table.
    pub fn mutexes(&self) -> &MutexTable {
        &self.state.mutexes
    }

    /// Mutable view of [`Self::mutexes`].
    pub fn mutexes_mut(&mut self) -> &mut MutexTable {
        &mut self.state.mutexes
    }

    /// Semaphore table.
    pub fn semaphores(&self) -> &SemaphoreTable {
        &self.state.semaphores
    }

    /// Mutable view of [`Self::semaphores`].
    pub fn semaphores_mut(&mut self) -> &mut SemaphoreTable {
        &mut self.state.semaphores
    }

    /// Event-queue table.
    pub fn event_queues(&self) -> &EventQueueTable {
        &self.state.event_queues
    }

    /// Mutable view of [`Self::event_queues`].
    pub fn event_queues_mut(&mut self) -> &mut EventQueueTable {
        &mut self.state.event_queues
    }

    /// Event-flag table.
    pub fn event_flags(&self) -> &EventFlagTable {
        &self.state.event_flags
    }

    /// Condition-variable table.
    pub fn conds(&self) -> &CondTable {
        &self.state.conds
    }

    /// Mutable view of [`Self::conds`].
    pub fn conds_mut(&mut self) -> &mut CondTable {
        &mut self.state.conds
    }

    /// Mutable view of [`Self::event_flags`].
    pub fn event_flags_mut(&mut self) -> &mut EventFlagTable {
        &mut self.state.event_flags
    }

    /// Allocate a child-thread stack of `size` bytes at `align`.
    pub fn allocate_child_stack(&mut self, size: u64, align: u64) -> Option<ThreadStack> {
        self.state.stack_allocator.allocate(size, align)
    }

    /// Bind an SPU `unit_id` to `(group_id, slot)`.
    pub fn record_spu(
        &mut self,
        unit_id: cellgov_event::UnitId,
        group_id: u32,
        slot: u32,
    ) -> Result<(), crate::thread_group::RecordSpuError> {
        self.state.groups.record_spu(unit_id, group_id, slot)
    }

    /// `Ok(Some(group_id))` when this notify drove the group to
    /// `Finished`.
    pub fn notify_spu_finished(
        &mut self,
        unit_id: cellgov_event::UnitId,
    ) -> Result<Option<u32>, crate::thread_group::NotifySpuFinishedError> {
        self.state.groups.notify_spu_finished(unit_id)
    }
}

#[cfg(test)]
#[path = "tests/lv2_host_tests.rs"]
mod tests;
