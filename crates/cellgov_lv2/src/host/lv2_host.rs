//! `Lv2Host` model, its `FirmwareIdentity` payload, and the state
//! primitives exposed to the dispatch submodules.

use std::collections::BTreeMap;

use cellgov_event::UnitId;
use cellgov_ps3_abi::elf::SYS_PROCESS_PARAM_SDK_VERSION_UNKNOWN;
use cellgov_time::GuestTicks;

use crate::fs_store::{FsMountTable, FsStore};
use crate::image::ContentStore;
use crate::ppu_thread::{
    PpuThread, PpuThreadAttrs, PpuThreadId, PpuThreadTable, ThreadStack, ThreadStackAllocator,
    TlsTemplate,
};
use crate::prx_registry::LoadedPrxRegistry;
use crate::sync_primitives::{
    CondTable, EventFlagTable, EventQueueTable, LwMutexTable, MutexTable, SemaphoreTable,
};
use crate::thread_group::ThreadGroupTable;

use super::mmapper::{MmapperHandleTable, PendingRegionInstall, SystemStateSeed};
use super::process;
use super::rsx::SysRsxContext;

/// LV2 host model driven by [`Self::dispatch`].
#[derive(Debug, Clone)]
pub struct Lv2Host {
    pub(super) content: ContentStore,
    pub(super) groups: ThreadGroupTable,
    pub(super) ppu_threads: PpuThreadTable,
    pub(super) tls_template: TlsTemplate,
    pub(super) stack_allocator: ThreadStackAllocator,
    /// Shared id allocator for mutex / semaphore / event-queue /
    /// event-flag / cond. `lwmutexes` has its own allocator from 1.
    pub(super) next_kernel_id: u32,
    pub(super) mem_alloc_ptr: u32,
    /// Bump cursor for `sys_mmapper_allocate_address` (256 MiB+ chunks).
    pub(super) mmapper_addr_cursor: u32,
    pub(super) rsx_mem_alloc_ptr: u32,
    pub(super) rsx_mem_handle_counter: u32,
    pub(super) rsx_context: SysRsxContext,
    /// Populated by 332 / 362, consumed by 334 / 337.
    pub(super) mmapper_handles: MmapperHandleTable,
    /// `ipc_key -> mem_id` for process-shared mmapper allocations.
    /// A keyed 332 with a registered key returns the existing
    /// `mem_id` (RPCS3's SYS_SYNC_NOT_CARE path,
    /// `sys_mmapper.cpp:103-128`); an unregistered key mints and
    /// registers. Not folded into [`Self::state_hash`]: the mint path
    /// advances `next_kernel_id` (hashed) and the found path mutates
    /// nothing.
    pub(super) mmapper_ipc: BTreeMap<u64, u32>,
    /// Boot-registered seeds keyed by `shm_ipc_key`; immutable after
    /// boot. Applied at most once, on the first 334 / 337 map of the
    /// matching shm. Not folded into [`Self::state_hash`]: the seed
    /// writes settle via `GuestMemory` (hashed there).
    pub(super) system_state_seeds: BTreeMap<u64, SystemStateSeed>,
    /// Keys whose seed has been applied. Not folded into
    /// [`Self::state_hash`] (the applied writes are; this set only
    /// suppresses re-application on a re-map).
    pub(super) system_seeds_applied: std::collections::BTreeSet<u64>,
    /// `shm_ipc_key -> mapped guest base` recorded when a seed is
    /// applied. The cond ring-check wake reads slot state through
    /// this base. Not hashed (derived from the hashed map effects).
    pub(super) system_seed_bases: BTreeMap<u64, u32>,
    /// `cond id -> create-time ipc_key` for process-shared conds
    /// (key 0 entries are not stored). Not hashed: create-time
    /// config mirrored from guest memory.
    pub(super) cond_ipc_keys: BTreeMap<u32, u64>,
    /// Witness: times the cond\[1\] ring-check arm satisfied a wait
    /// immediately. Expected 0 under the V256 seed; a non-zero
    /// value means a refill wait observed a non-depleted ring.
    /// Not hashed (instrument-only).
    pub(super) cond_ring_wakes: u64,
    /// Witness: parks on a cellSysutil cond\[0\] -- the producer-fed
    /// record-finish waits CellGov has no producer for -- keyed by
    /// slot index. Not hashed (instrument-only).
    pub(super) cond0_producer_waits_by_slot: BTreeMap<u64, u64>,
    /// Witness: `sys_cond_signal` dispatch count (drain witness for
    /// the seeded-ring consumer). Not hashed (instrument-only).
    pub(super) cond_signal_dispatches: u64,
    /// Witness: `sys_cond_signal` dispatches keyed by the target
    /// cond's create-time ipc_key (keyed conds only). Per-slot /
    /// per-facility drain attribution for the seeded-ring consumer.
    /// Not hashed (instrument-only).
    pub(super) cond_keyed_signal_counts: BTreeMap<u64, u64>,
    /// Pending region-install requests emitted by 334 / 337. Drained
    /// by the runtime post-dispatch and applied to `GuestMemory`
    /// before the dispatch's effects commit. Not folded into
    /// [`Self::state_hash`]; the handle table itself carries the
    /// hashable state.
    pub(super) pending_region_installs: Vec<PendingRegionInstall>,
    /// Authoritative ledger of every mmapper-window range this host
    /// has handed out via 334 / 337 (and not yet released). The
    /// search in 337 consults this to find a free aligned range
    /// at-or-after the caller's hint. Recorded as
    /// `(start_addr -> size)` keyed in BTreeMap order so the
    /// nearest-below lookup is `O(log n)`. Not folded into
    /// [`Self::state_hash`] -- the install side-effect is what
    /// settles via `GuestMemory`; this ledger is the host's local
    /// witness of what was minted, used to keep the search and the
    /// install coherent.
    pub(super) mmapper_install_ledger: BTreeMap<u32, u32>,
    pub(super) lwmutexes: LwMutexTable,
    pub(super) mutexes: MutexTable,
    pub(super) semaphores: SemaphoreTable,
    pub(super) event_queues: EventQueueTable,
    pub(super) event_flags: EventFlagTable,
    pub(super) conds: CondTable,
    /// Dispatch-local scratch; not folded into [`Self::state_hash`].
    pub(super) current_tick: GuestTicks,
    /// Running count of host-invariant breaks. Not hashed.
    pub(super) invariant_break_count: usize,
    /// Drained after each `Lv2Host::dispatch` by the runtime. Not
    /// folded into [`Self::state_hash`].
    pub(super) pending_invariant_breaks: Vec<super::diagnostics::InvariantBreakReason>,
    /// Captured `sys_tty_write` byte stream. Not folded into
    /// [`Self::state_hash`].
    pub(super) tty_log: Vec<u8>,
    /// Feeds `sys_process_get_number_of_object`. Not folded into
    /// [`Self::state_hash`].
    pub(super) process_counts: process::ProcessCounts,
    pub(super) fs_store: FsStore,
    /// Consulted by the dispatch layer when a guest path is not
    /// pre-registered in [`Self::fs_store`]. Boot populates from the
    /// title manifest; immutable thereafter.
    pub(super) fs_mounts: FsMountTable,
    /// Minimum viable PRX set loaded at boot. Empty when no
    /// firmware-dir was configured.
    pub(super) prx_registry: LoadedPrxRegistry,
    /// Per-thread count of distinct lwmutexes held. Recursive
    /// re-acquires of the same lwmutex do not bump the count; only
    /// first-acquire (FREE -> me) and kernel-side transfer
    /// (LwMutexWake) do.
    pub(super) lwmutex_holds: BTreeMap<PpuThreadId, u32>,
    /// Folded into [`Self::state_hash`] when set.
    pub(super) firmware_identity: Option<FirmwareIdentity>,
    /// Audit C-5b witness: count of `dispatch_thread_initialize`
    /// invocations. The catch-all `debug_assert!` at
    /// `host/spu.rs:235` guards against being called with the
    /// wrong request variant; silence is non-vacuous only when
    /// the dispatch actually ran. Not folded into
    /// [`Self::state_hash`] (instrument-only).
    pub(super) spu_thread_initialize_dispatches: u64,
    /// Audit C-6c witness: count of `cond_reacquire_wake` calls.
    /// The `debug_assert!(!use_lwmutex, ...)` at `host/cond.rs:247`
    /// guards against an unimplemented lwmutex-cond re-acquire
    /// path; silence is non-vacuous only when the function ran.
    /// Not folded into [`Self::state_hash`].
    pub(super) cond_reacquire_wake_calls: u64,
    /// `sys_process_get_sdk_version` return value. Read from the
    /// title ELF's `sys_proc_param` segment at boot (see
    /// `cellgov_ppu::loader::find_sys_process_param`). RPCS3 mirror:
    /// `g_ps3_process_info.sdk_ver` set from the LOOS+1 program
    /// header's `process_param_t.sdk_version`
    /// (rpcs3 PPUModule.cpp). The PS3 sentinel for
    /// absent param segment is `0xFFFFFFFF` (the same one PSL1GHT
    /// homebrew sees); this default matches that contract for
    /// callers that never invoke `set_sdk_version`.
    pub(super) sdk_version: u32,
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
            system_state_seeds: BTreeMap::new(),
            system_seeds_applied: std::collections::BTreeSet::new(),
            system_seed_bases: BTreeMap::new(),
            cond_ipc_keys: BTreeMap::new(),
            cond_ring_wakes: 0,
            cond0_producer_waits_by_slot: BTreeMap::new(),
            cond_signal_dispatches: 0,
            cond_keyed_signal_counts: BTreeMap::new(),
            pending_region_installs: Vec::new(),
            mmapper_install_ledger: BTreeMap::new(),
            lwmutexes: LwMutexTable::new(),
            mutexes: MutexTable::new(),
            semaphores: SemaphoreTable::new(),
            event_queues: EventQueueTable::new(),
            event_flags: EventFlagTable::new(),
            conds: CondTable::new(),
            current_tick: GuestTicks::ZERO,
            invariant_break_count: 0,
            pending_invariant_breaks: Vec::new(),
            tty_log: Vec::new(),
            process_counts: process::ProcessCounts::new(),
            fs_store,
            fs_mounts: FsMountTable::new(),
            prx_registry: LoadedPrxRegistry::new(),
            lwmutex_holds: BTreeMap::new(),
            firmware_identity: None,
            spu_thread_initialize_dispatches: 0,
            cond_reacquire_wake_calls: 0,
            sdk_version: SYS_PROCESS_PARAM_SDK_VERSION_UNKNOWN,
        }
    }

    /// Set the title's recorded SDK version (the value read from the
    /// title ELF's `process_param_t`). Boot reads it via
    /// `cellgov_ppu::loader::find_sys_process_param` and plumbs the
    /// parsed `sdk_version` through. Callers that omit this leave the
    /// PS3 absent-case sentinel `0xFFFFFFFF` in place.
    pub fn set_sdk_version(&mut self, sdk_version: u32) {
        self.sdk_version = sdk_version;
    }

    /// The value `sys_process_get_sdk_version` will write into the
    /// caller's `version_out_ptr`.
    #[inline]
    pub fn sdk_version(&self) -> u32 {
        self.sdk_version
    }

    /// Audit C-5b witness: count of `dispatch_thread_initialize`
    /// invocations. See the field doc on
    /// `spu_thread_initialize_dispatches`.
    #[inline]
    pub fn spu_thread_initialize_dispatches(&self) -> u64 {
        self.spu_thread_initialize_dispatches
    }

    /// Audit C-6c witness: count of `cond_reacquire_wake` calls.
    /// See the field doc on `cond_reacquire_wake_calls`.
    #[inline]
    pub fn cond_reacquire_wake_calls(&self) -> u64 {
        self.cond_reacquire_wake_calls
    }

    /// Record the verified-firmware identity. Boot is one-shot; a
    /// second call panics in debug builds.
    pub fn set_firmware_identity(&mut self, image_version: &str, pup_sha256_bytes: [u8; 32]) {
        debug_assert!(
            self.firmware_identity.is_none(),
            "firmware identity already set; boot is one-shot",
        );
        let mut h = cellgov_mem::Fnv1aHasher::new();
        h.write(image_version.as_bytes());
        self.firmware_identity = Some(FirmwareIdentity {
            image_version_hash: h.finish(),
            pup_sha256_bytes,
        });
    }

    /// `None` until boot records one.
    pub fn firmware_identity(&self) -> Option<&FirmwareIdentity> {
        self.firmware_identity.as_ref()
    }

    /// In-memory filesystem store.
    pub fn fs_store(&self) -> &FsStore {
        &self.fs_store
    }

    /// Mutable view of [`Self::fs_store`].
    pub fn fs_store_mut(&mut self) -> &mut FsStore {
        &mut self.fs_store
    }

    /// Guest-path to host-path mount table.
    pub fn fs_mounts(&self) -> &FsMountTable {
        &self.fs_mounts
    }

    /// Mutable view; written by boot only.
    pub fn fs_mounts_mut(&mut self) -> &mut FsMountTable {
        &mut self.fs_mounts
    }

    /// Distinct lwmutexes currently held by `tid`.
    pub fn lwmutex_holds_for(&self, tid: PpuThreadId) -> u32 {
        self.lwmutex_holds.get(&tid).copied().unwrap_or(0)
    }

    /// Bumps the count for a first-acquire (FREE -> tid) or a
    /// kernel-side transfer. Recursive re-acquires (tid already
    /// the owner) are tracked elsewhere and must not pass through
    /// this entry.
    pub fn lwmutex_holds_inc(&mut self, tid: PpuThreadId) {
        let slot = self.lwmutex_holds.entry(tid).or_insert(0);
        debug_assert!(*slot < u32::MAX, "lwmutex hold count overflow on {tid:?}",);
        *slot += 1;
    }

    /// Release builds saturate at 0 so a leak does not corrupt
    /// downstream counters.
    pub fn lwmutex_holds_dec(&mut self, tid: PpuThreadId) {
        if let Some(slot) = self.lwmutex_holds.get_mut(&tid) {
            debug_assert!(*slot > 0, "lwmutex hold count underflow on {tid:?}",);
            *slot = slot.saturating_sub(1);
            if *slot == 0 {
                self.lwmutex_holds.remove(&tid);
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
        self.lwmutex_holds.remove(&tid);
    }

    /// `false` when `unit` has no PPU thread mapping.
    pub fn unit_holds_lwmutex(&self, unit: UnitId) -> bool {
        match self.ppu_threads.thread_id_for_unit(unit) {
            Some(tid) => self.lwmutex_holds_for(tid) > 0,
            None => false,
        }
    }

    /// See [`process::ProcessCounts::fs_fd_inc`] for the no-decrement
    /// contract.
    pub(super) fn fs_fd_count_inc(&mut self) {
        self.process_counts.fs_fd_inc();
    }

    /// Increment the live `sys_lwcond` object count.
    pub fn lwcond_count_inc(&mut self) {
        self.process_counts.lwcond_inc();
    }

    /// Decrement the live `sys_lwcond` count; saturates at 0.
    pub fn lwcond_count_dec(&mut self) {
        self.process_counts.lwcond_dec();
    }

    /// `sys_tty_write` byte stream in dispatch order.
    #[inline]
    pub fn tty_log(&self) -> &[u8] {
        &self.tty_log
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
        self.mem_alloc_ptr = base;
    }

    /// sys_rsx host context.
    #[inline]
    pub fn sys_rsx_context(&self) -> &SysRsxContext {
        &self.rsx_context
    }

    /// Record an iomap mapping without going through 672. Synthetic
    /// test scenarios use this to wire up the IO -> EA translation
    /// the FIFO advance pass needs without booting the firmware-set
    /// `sys_rsx_context_iomap` path. Matches the
    /// `seed_primary_ppu_thread` pattern; production code calls
    /// `dispatch_sys_rsx_context_iomap` which validates against the
    /// 672 contract.
    pub fn seed_rsx_iomap(&mut self, io: u32, ea: u32, size: u32) {
        self.rsx_context.iomap_io = io;
        self.rsx_context.iomap_ea = ea;
        self.rsx_context.iomap_size = size;
    }

    /// Mark the sys_rsx context as allocated under `context_id`
    /// without going through 670. Synthetic test scenarios use this
    /// to satisfy the `allocated && matching id` guard at the top of
    /// `sys_rsx_context_attribute` (674) without the OUT-pointer
    /// memory plumbing 670 requires. Matches `seed_rsx_iomap`'s
    /// shape; production code calls `dispatch_sys_rsx_context_allocate`.
    pub fn seed_rsx_context_allocated(&mut self, context_id: u32) {
        self.rsx_context.allocated = true;
        self.rsx_context.context_id = context_id;
    }

    pub(super) fn alloc_id(&mut self) -> u32 {
        let id = self.next_kernel_id;
        self.next_kernel_id = self
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
        let base = self.mmapper_addr_cursor;
        let next = base.checked_add(rounded)?;
        if next > Self::MMAPPER_REGION_END {
            return None;
        }
        self.mmapper_addr_cursor = next;
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
    /// `tools/rpcs3-src/rpcs3/Emu/Cell/lv2/sys_mmapper.cpp:696` is
    /// area selection, not in-area alignment).
    ///
    /// Returns `None` on exhaustion (matches RPCS3's `CELL_ENOMEM`
    /// path at `sys_mmapper.cpp:753`).
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
        let prior = self.mmapper_install_ledger.insert(addr, size);
        debug_assert!(
            prior.is_none(),
            "mmapper ledger: addr {addr:#x} already recorded (size {prior:?})",
        );
    }

    /// `ipc_key -> mem_id` registrations made by keyed 332 calls.
    pub fn mmapper_ipc(&self) -> &BTreeMap<u64, u32> {
        &self.mmapper_ipc
    }

    /// Register a boot-state seed; a duplicate `shm_ipc_key` replaces
    /// the prior entry (last-write-wins). Boot-only: registering
    /// after the matching shm has been mapped has no effect.
    pub fn register_system_seed(&mut self, seed: SystemStateSeed) {
        self.system_state_seeds.insert(seed.shm_ipc_key, seed);
    }

    /// Boot-registered seeds keyed by `shm_ipc_key`.
    pub fn system_state_seeds(&self) -> &BTreeMap<u64, SystemStateSeed> {
        &self.system_state_seeds
    }

    /// `true` once the seed registered under `shm_ipc_key` has been
    /// applied by a 334 / 337 map.
    pub fn system_seed_applied(&self, shm_ipc_key: u64) -> bool {
        self.system_seeds_applied.contains(&shm_ipc_key)
    }

    /// Mapped guest base of the seeded shm, once applied.
    pub fn system_seed_base(&self, shm_ipc_key: u64) -> Option<u32> {
        self.system_seed_bases.get(&shm_ipc_key).copied()
    }

    /// Witness: times the cond\[1\] ring-check arm satisfied a wait
    /// immediately.
    #[inline]
    pub fn cond_ring_wakes(&self) -> u64 {
        self.cond_ring_wakes
    }

    /// Witness: parks on a cellSysutil cond\[0\] record-finish wait,
    /// summed over slots.
    pub fn cond0_producer_waits(&self) -> u64 {
        self.cond0_producer_waits_by_slot.values().sum()
    }

    /// Witness: cond\[0\] producer-wait parks keyed by slot index.
    pub fn cond0_producer_waits_by_slot(&self) -> &BTreeMap<u64, u64> {
        &self.cond0_producer_waits_by_slot
    }

    /// Witness: `sys_cond_signal` dispatch count.
    #[inline]
    pub fn cond_signal_dispatches(&self) -> u64 {
        self.cond_signal_dispatches
    }

    /// Witness: `sys_cond_signal` dispatches keyed by the target
    /// cond's create-time ipc_key.
    pub fn cond_keyed_signal_counts(&self) -> &BTreeMap<u64, u64> {
        &self.cond_keyed_signal_counts
    }

    /// Read-only `pending_region_installs` snapshot used by sibling
    /// dispatch-arm tests. Not a drain; the runtime is still the
    /// authoritative drain consumer.
    #[cfg(all(test, debug_assertions))]
    pub(super) fn drain_pending_region_installs_inspect(&self) -> &[PendingRegionInstall] {
        &self.pending_region_installs
    }

    /// Per-title content manifest store.
    pub fn content_store(&self) -> &ContentStore {
        &self.content
    }

    /// Mutable view of [`Self::content_store`].
    pub fn content_store_mut(&mut self) -> &mut ContentStore {
        &mut self.content
    }

    /// Loaded-PRX registry.
    pub fn prx_registry(&self) -> &LoadedPrxRegistry {
        &self.prx_registry
    }

    /// Mutable view of [`Self::prx_registry`].
    pub fn prx_registry_mut(&mut self) -> &mut LoadedPrxRegistry {
        &mut self.prx_registry
    }

    /// SPU thread-group table.
    pub fn thread_groups(&self) -> &ThreadGroupTable {
        &self.groups
    }

    /// Mutable view of [`Self::thread_groups`].
    pub fn thread_groups_mut(&mut self) -> &mut ThreadGroupTable {
        &mut self.groups
    }

    /// PPU thread table.
    pub fn ppu_threads(&self) -> &PpuThreadTable {
        &self.ppu_threads
    }

    /// Mutable view of [`Self::ppu_threads`].
    pub fn ppu_threads_mut(&mut self) -> &mut PpuThreadTable {
        &mut self.ppu_threads
    }

    /// Call exactly once after the primary PPU unit is registered.
    pub fn seed_primary_ppu_thread(&mut self, unit_id: UnitId, attrs: PpuThreadAttrs) {
        self.ppu_threads.insert_primary(unit_id, attrs);
    }

    /// Alias a transient unit (e.g. a per-module module_start unit)
    /// to the primary thread so sync-syscall dispatch resolves the
    /// caller. Mirrors real LV2's "module_start runs on the calling
    /// thread" contract. See
    /// [`PpuThreadTable::alias_unit`][crate::ppu_thread::PpuThreadTable::alias_unit].
    pub fn alias_unit_to_primary(&mut self, unit_id: UnitId) -> bool {
        self.ppu_threads.alias_unit(unit_id, PpuThreadId::PRIMARY)
    }

    /// Drop an alias previously installed via [`Self::alias_unit_to_primary`].
    pub fn drop_ppu_thread_alias(&mut self, unit_id: UnitId) -> bool {
        self.ppu_threads.drop_alias(unit_id)
    }

    /// PPU thread record bound to `unit_id`, if any.
    pub fn ppu_thread_for_unit(&self, unit_id: UnitId) -> Option<&PpuThread> {
        self.ppu_threads.get_by_unit(unit_id)
    }

    /// PPU thread id bound to `unit_id`, if any.
    pub fn ppu_thread_id_for_unit(&self, unit_id: UnitId) -> Option<PpuThreadId> {
        self.ppu_threads.thread_id_for_unit(unit_id)
    }

    /// `false` when `unit_id` has no PPU mapping.
    pub fn is_ppu_thread_finished_for_unit(&self, unit_id: UnitId) -> bool {
        match self.ppu_threads.get_by_unit(unit_id) {
            Some(thread) => thread.state.is_finished(),
            None => false,
        }
    }

    /// Install the TLS template used for new PPU threads.
    pub fn set_tls_template(&mut self, template: TlsTemplate) {
        self.tls_template = template;
    }

    /// Installed TLS template.
    pub fn tls_template(&self) -> &TlsTemplate {
        &self.tls_template
    }

    /// Lightweight mutex table.
    pub fn lwmutexes(&self) -> &LwMutexTable {
        &self.lwmutexes
    }

    /// Mutable view of [`Self::lwmutexes`].
    pub fn lwmutexes_mut(&mut self) -> &mut LwMutexTable {
        &mut self.lwmutexes
    }

    /// Mutex table.
    pub fn mutexes(&self) -> &MutexTable {
        &self.mutexes
    }

    /// Mutable view of [`Self::mutexes`].
    pub fn mutexes_mut(&mut self) -> &mut MutexTable {
        &mut self.mutexes
    }

    /// Semaphore table.
    pub fn semaphores(&self) -> &SemaphoreTable {
        &self.semaphores
    }

    /// Mutable view of [`Self::semaphores`].
    pub fn semaphores_mut(&mut self) -> &mut SemaphoreTable {
        &mut self.semaphores
    }

    /// Event-queue table.
    pub fn event_queues(&self) -> &EventQueueTable {
        &self.event_queues
    }

    /// Mutable view of [`Self::event_queues`].
    pub fn event_queues_mut(&mut self) -> &mut EventQueueTable {
        &mut self.event_queues
    }

    /// Event-flag table.
    pub fn event_flags(&self) -> &EventFlagTable {
        &self.event_flags
    }

    /// Condition-variable table.
    pub fn conds(&self) -> &CondTable {
        &self.conds
    }

    /// Mutable view of [`Self::conds`].
    pub fn conds_mut(&mut self) -> &mut CondTable {
        &mut self.conds
    }

    /// Mutable view of [`Self::event_flags`].
    pub fn event_flags_mut(&mut self) -> &mut EventFlagTable {
        &mut self.event_flags
    }

    /// Allocate a child-thread stack of `size` bytes at `align`.
    pub fn allocate_child_stack(&mut self, size: u64, align: u64) -> Option<ThreadStack> {
        self.stack_allocator.allocate(size, align)
    }

    /// Bind an SPU `unit_id` to `(group_id, slot)`.
    pub fn record_spu(
        &mut self,
        unit_id: cellgov_event::UnitId,
        group_id: u32,
        slot: u32,
    ) -> Result<(), crate::thread_group::RecordSpuError> {
        self.groups.record_spu(unit_id, group_id, slot)
    }

    /// `Ok(Some(group_id))` when this notify drove the group to
    /// `Finished`.
    pub fn notify_spu_finished(
        &mut self,
        unit_id: cellgov_event::UnitId,
    ) -> Result<Option<u32>, crate::thread_group::NotifySpuFinishedError> {
        self.groups.notify_spu_finished(unit_id)
    }
}

#[cfg(test)]
#[path = "tests/lv2_host_tests.rs"]
mod tests;
