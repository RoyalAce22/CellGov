//! `Lv2Host` model, its `FirmwareIdentity` payload, and the state
//! primitives exposed to the dispatch submodules.

use std::collections::BTreeMap;

use cellgov_event::UnitId;
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

use super::mmapper::{MmapperHandleTable, PendingRegionInstall};
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
    /// Bump cursor for `sys_mmapper_allocate_address`. Separate
    /// from `mem_alloc_ptr` because mmapper grants 256 MiB+ chunks
    /// while `sys_memory_allocate` deals in 64K-aligned <=1 MiB
    /// slices.
    pub(super) mmapper_addr_cursor: u32,
    /// Separate bump cursor so the RSX-visible region cannot collide
    /// with PPU allocations.
    pub(super) rsx_mem_alloc_ptr: u32,
    pub(super) rsx_mem_handle_counter: u32,
    pub(super) rsx_context: SysRsxContext,
    /// Shared-memory handle table populated by `sys_mmapper_allocate_shared_memory`
    /// (332) and `sys_mmapper_allocate_shared_memory_from_container`
    /// (362), consumed by `sys_mmapper_map_shared_memory` (334).
    pub(super) mmapper_handles: MmapperHandleTable,
    /// Pending region-install requests emitted by 334. Drained by the
    /// runtime post-dispatch and applied to `GuestMemory` before the
    /// dispatch's effects commit (so effects can target the new region).
    /// Not folded into [`Self::state_hash`]; the handle table itself
    /// carries the hashable state.
    pub(super) pending_region_installs: Vec<PendingRegionInstall>,
    pub(super) lwmutexes: LwMutexTable,
    pub(super) mutexes: MutexTable,
    pub(super) semaphores: SemaphoreTable,
    pub(super) event_queues: EventQueueTable,
    pub(super) event_flags: EventFlagTable,
    pub(super) conds: CondTable,
    /// Dispatch-local scratch; not folded into [`Self::state_hash`].
    pub(super) current_tick: GuestTicks,
    /// Running count of host-invariant breaks. Non-zero means at
    /// least one wake or table update fell back to a degraded
    /// response. Not hashed.
    pub(super) invariant_break_count: usize,
    /// Pending invariant-break events drained after each
    /// `Lv2Host::dispatch` by the runtime and emitted as
    /// `TraceRecord::HostInvariantBreak` records via the cross-crate
    /// bridge in `cellgov_core::runtime::trace_bridge`. Not folded
    /// into [`Self::state_hash`].
    pub(super) pending_invariant_breaks: Vec<super::diagnostics::InvariantBreakReason>,
    /// Captured `sys_tty_write` byte stream in dispatch order.
    /// Observation channel; not folded into [`Self::state_hash`].
    pub(super) tty_log: Vec<u8>,
    /// Live-object counters for primitives stubbed as ID allocators
    /// only; feed `sys_process_get_number_of_object`. Not folded
    /// into [`Self::state_hash`].
    pub(super) process_counts: process::ProcessCounts,
    pub(super) fs_store: FsStore,
    /// Consulted by the dispatch layer when a guest path is not
    /// pre-registered in [`Self::fs_store`]. Boot populates from the
    /// title manifest; immutable thereafter.
    pub(super) fs_mounts: FsMountTable,
    /// Minimum viable PRX set loaded at boot. Looked up by
    /// `_sys_prx_load_module` (path -> kernel id) and walked by
    /// `_sys_prx_get_module_list`. Empty when no firmware-dir was
    /// configured (e.g. synthetic-ELF harnesses).
    pub(super) prx_registry: LoadedPrxRegistry,
    /// Per-thread count of distinct lwmutexes held. Recursive
    /// re-acquires of the same lwmutex do not bump the count; only
    /// first-acquire (FREE -> me) and kernel-side transfer
    /// (LwMutexWake) do. Read by the runtime to drive
    /// critical-section-aware scheduler stickiness.
    pub(super) lwmutex_holds: BTreeMap<PpuThreadId, u32>,
    /// Firmware identity from `firmware.toml`. Folded into
    /// [`Self::state_hash`] when set.
    pub(super) firmware_identity: Option<FirmwareIdentity>,
}

/// Captured at boot via the verified `firmware.toml` manifest. The
/// `image_version` hash and `pup_sha256` together identify the PUP
/// the install came from; both fold into `Lv2Host::state_hash`.
#[derive(Debug, Clone)]
pub struct FirmwareIdentity {
    /// FNV-1a hash of the verified `image_version` string from
    /// `firmware.toml`. Pairs with `pup_sha256_bytes` to uniquely
    /// identify the PUP this install was decrypted from.
    pub image_version_hash: u64,
    /// Raw SHA-256 of the originating PUP file, copied verbatim
    /// from `firmware.toml`.
    pub pup_sha256_bytes: [u8; 32],
}

impl Default for Lv2Host {
    fn default() -> Self {
        Self::new()
    }
}

impl Lv2Host {
    /// Guest base of the 256 MB RSX-visible window. Sits above the
    /// user-memory bumper and below `MEM_ALLOC_REGION_END` so the
    /// two allocators cannot collide.
    pub const SYS_RSX_MEM_BASE: u32 = 0x3000_0000;

    /// Upper bound (exclusive) of the sys_rsx memory region.
    pub const SYS_RSX_MEM_END: u32 = Self::SYS_RSX_MEM_BASE + 0x1000_0000;

    /// Lower bound (inclusive) of the `sys_mmapper_allocate_address`
    /// handout window. Set to `0x5000_0000` -- 256 MiB above
    /// `SYS_RSX_MEM_END` -- so the reserved
    /// `[0x4000_0000, 0x5000_0000)` rsx_context window (covering
    /// `sys_rsx::device::RSX_DEVICE_ADDR` and any future
    /// per-context allocations placed in the same window) cannot
    /// alias an mmapper handout. RPCS3 achieves the same disjoint
    /// guarantee via `vm::reserve_map(vm::rsx_context, 0,
    /// 0x10000000, 0x403)` before either 670 or 675 allocates
    /// inside it.
    pub const MMAPPER_REGION_START: u32 = 0x5000_0000;

    /// Upper bound (exclusive) of the `sys_mmapper_allocate_address`
    /// region. Capped at `0xC000_0000` so the mmapper allocator
    /// cannot hand out an address that aliases the RSX dma_control
    /// MMIO region at `control_register::DMA_CONTROL_BASE`. Beyond
    /// the MMIO region sits the kernel-reserved PPU stack region.
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
            // Sits immediately above the sys_rsx-visible window so
            // mmapper grants and sys_rsx-region addresses never
            // alias. Capped at MMAPPER_REGION_END (0xD000_0000) so
            // mmapper grants cannot walk into the PPU stack region.
            mmapper_addr_cursor: Self::MMAPPER_REGION_START,
            rsx_mem_alloc_ptr: Self::SYS_RSX_MEM_BASE,
            rsx_mem_handle_counter: 1,
            rsx_context: SysRsxContext::new(),
            mmapper_handles: MmapperHandleTable::new(),
            pending_region_installs: Vec::new(),
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
        }
    }

    /// Record the verified-firmware identity from the CLI boot path.
    /// `image_version` is FNV-1a-hashed; `pup_sha256_bytes` is the
    /// 32-byte SHA-256 digest. Boot is one-shot; the no-overwrite
    /// invariant is asserted.
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

    /// Captured firmware identity, or `None` before boot recorded one.
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

    /// Mount table mapping guest paths to host paths.
    pub fn fs_mounts(&self) -> &FsMountTable {
        &self.fs_mounts
    }

    /// Mutable view of [`Self::fs_mounts`]; written by boot only.
    pub fn fs_mounts_mut(&mut self) -> &mut FsMountTable {
        &mut self.fs_mounts
    }

    /// Distinct lwmutexes currently held by `tid`.
    pub fn lwmutex_holds_for(&self, tid: PpuThreadId) -> u32 {
        self.lwmutex_holds.get(&tid).copied().unwrap_or(0)
    }

    /// # Contract
    ///
    /// Bumps the count for a first-acquire (FREE -> tid) or a
    /// kernel-side transfer. Recursive re-acquires (tid already
    /// the owner) are tracked elsewhere and must not pass through
    /// this entry. Overflow at `u32::MAX` is physically impossible
    /// from legitimate guest behaviour, so the upper bound is
    /// asserted rather than saturated.
    pub fn lwmutex_holds_inc(&mut self, tid: PpuThreadId) {
        let slot = self.lwmutex_holds.entry(tid).or_insert(0);
        debug_assert!(*slot < u32::MAX, "lwmutex hold count overflow on {tid:?}",);
        *slot += 1;
    }

    /// Debug-asserts that the count is non-zero; release builds
    /// saturate at 0 so a leak does not corrupt downstream counters.
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

    /// Drop any tracked count for `tid`; used at thread-exit and
    /// stale-owner recovery so a dead thread's count does not leak.
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

    /// Forwarder onto [`process::ProcessCounts::fs_fd_inc`]; see
    /// that method for the no-decrement contract.
    pub(super) fn fs_fd_count_inc(&mut self) {
        self.process_counts.fs_fd_inc();
    }

    /// Increment the live `sys_lwcond` object count.
    pub fn lwcond_count_inc(&mut self) {
        self.process_counts.lwcond_inc();
    }

    /// Decrement the live `sys_lwcond` object count, saturating at 0.
    pub fn lwcond_count_dec(&mut self) {
        self.process_counts.lwcond_dec();
    }

    /// Captured `sys_tty_write` byte stream in dispatch order.
    #[inline]
    pub fn tty_log(&self) -> &[u8] {
        &self.tty_log
    }

    /// Callers that load a real ELF must set this to the
    /// 64KB-aligned address above the ELF's highest PT_LOAD end;
    /// the default (`0x0001_0000`) assumes no ELF is loaded and
    /// will overwrite the image otherwise.
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

    pub(super) fn alloc_id(&mut self) -> u32 {
        let id = self.next_kernel_id;
        self.next_kernel_id = self
            .next_kernel_id
            .checked_add(1)
            .expect("kernel id space exhausted");
        id
    }

    /// Bump the mmapper VM cursor by `size` rounded up to the
    /// 256 MiB sys_mmapper_allocate_address granule and return the
    /// pre-bump cursor as the allocation address. Returns `None`
    /// for `size == 0` (LV2 returns EINVAL there), when the bump
    /// would overflow `u32`, or when the resulting range would
    /// cross [`Self::MMAPPER_REGION_END`] into the kernel-reserved
    /// PPU stack region.
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

    /// Boot uses this to register the minimum viable PRX set after
    /// `load_firmware_set` returns.
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

    /// Look up the PPU thread record bound to `unit_id`, if any.
    pub fn ppu_thread_for_unit(&self, unit_id: UnitId) -> Option<&PpuThread> {
        self.ppu_threads.get_by_unit(unit_id)
    }

    /// Look up the PPU thread id bound to `unit_id`, if any.
    pub fn ppu_thread_id_for_unit(&self, unit_id: UnitId) -> Option<PpuThreadId> {
        self.ppu_threads.thread_id_for_unit(unit_id)
    }

    /// `true` only when `unit_id` maps to a `PpuThread` whose state
    /// is [`crate::ppu_thread::PpuThreadState::Finished`]. A unit with
    /// no PPU mapping returns `false`.
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

    /// Bind an SPU `unit_id` to `(group_id, slot)` in the group table.
    pub fn record_spu(
        &mut self,
        unit_id: cellgov_event::UnitId,
        group_id: u32,
        slot: u32,
    ) -> Result<(), crate::thread_group::RecordSpuError> {
        self.groups.record_spu(unit_id, group_id, slot)
    }

    /// Returns `Ok(Some(group_id))` when this notify drove the
    /// group to `Finished`.
    pub fn notify_spu_finished(
        &mut self,
        unit_id: cellgov_event::UnitId,
    ) -> Result<Option<u32>, crate::thread_group::NotifySpuFinishedError> {
        self.groups.notify_spu_finished(unit_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::test_support::primary_attrs;

    #[test]
    fn new_host_has_empty_ppu_thread_table() {
        let host = Lv2Host::new();
        assert!(host.ppu_threads().is_empty());
        assert!(host.ppu_thread_for_unit(UnitId::new(0)).is_none());
        assert!(host.ppu_thread_id_for_unit(UnitId::new(0)).is_none());
    }

    #[test]
    fn seed_primary_ppu_thread_records_mapping() {
        let mut host = Lv2Host::new();
        host.seed_primary_ppu_thread(UnitId::new(0), primary_attrs());
        assert_eq!(host.ppu_threads().len(), 1);
        let primary = host.ppu_thread_for_unit(UnitId::new(0)).unwrap();
        assert_eq!(primary.id, PpuThreadId::PRIMARY);
        assert_eq!(primary.unit_id, UnitId::new(0));
        assert_eq!(primary.state, crate::ppu_thread::PpuThreadState::Runnable);
        assert_eq!(
            host.ppu_thread_id_for_unit(UnitId::new(0)),
            Some(PpuThreadId::PRIMARY),
        );
    }

    // Not gated on `cfg(debug_assertions)`: the underlying check in
    // `PpuThreadTable::insert_primary` is `assert!`, not
    // `debug_assert!`, so the panic fires in both debug and release
    // builds.
    #[test]
    #[should_panic(expected = "primary thread already inserted")]
    fn seeding_primary_twice_panics() {
        let mut host = Lv2Host::new();
        host.seed_primary_ppu_thread(UnitId::new(0), primary_attrs());
        host.seed_primary_ppu_thread(UnitId::new(1), primary_attrs());
    }

    #[test]
    fn set_tls_template_stores_bytes() {
        let mut host = Lv2Host::new();
        assert!(host.tls_template().is_empty());
        host.set_tls_template(crate::ppu_thread::TlsTemplate::new(
            vec![0xDE, 0xAD],
            0x100,
            0x10,
            0x89_5cd0,
        ));
        let tpl = host.tls_template();
        assert!(!tpl.is_empty());
        assert_eq!(tpl.initial_bytes(), &[0xDE, 0xAD]);
        assert_eq!(tpl.vaddr(), 0x89_5cd0);
    }

    #[test]
    fn lwmutex_holds_inc_increments_per_thread() {
        let mut host = Lv2Host::new();
        let a = PpuThreadId::new(0x0100_0001);
        let b = PpuThreadId::new(0x0100_0002);
        assert_eq!(host.lwmutex_holds_for(a), 0);
        host.lwmutex_holds_inc(a);
        host.lwmutex_holds_inc(a);
        host.lwmutex_holds_inc(b);
        assert_eq!(host.lwmutex_holds_for(a), 2);
        assert_eq!(host.lwmutex_holds_for(b), 1);
    }

    #[test]
    fn lwmutex_holds_dec_zeroes_and_drops_entry() {
        let mut host = Lv2Host::new();
        let a = PpuThreadId::new(0x0100_0001);
        host.lwmutex_holds_inc(a);
        host.lwmutex_holds_inc(a);
        host.lwmutex_holds_dec(a);
        assert_eq!(host.lwmutex_holds_for(a), 1);
        host.lwmutex_holds_dec(a);
        assert_eq!(host.lwmutex_holds_for(a), 0);
    }

    #[test]
    fn lwmutex_holds_clear_removes_entry() {
        let mut host = Lv2Host::new();
        let a = PpuThreadId::new(0x0100_0001);
        host.lwmutex_holds_inc(a);
        host.lwmutex_holds_inc(a);
        host.lwmutex_holds_clear(a);
        assert_eq!(host.lwmutex_holds_for(a), 0);
    }

    #[test]
    fn unit_holds_lwmutex_via_thread_table() {
        let mut host = Lv2Host::new();
        let unit = UnitId::new(0);
        host.seed_primary_ppu_thread(unit, primary_attrs());
        assert!(!host.unit_holds_lwmutex(unit));
        let tid = host.ppu_thread_id_for_unit(unit).unwrap();
        host.lwmutex_holds_inc(tid);
        assert!(host.unit_holds_lwmutex(unit));
        host.lwmutex_holds_dec(tid);
        assert!(!host.unit_holds_lwmutex(unit));
    }

    #[test]
    fn unit_holds_lwmutex_unmapped_unit_is_false() {
        let host = Lv2Host::new();
        assert!(!host.unit_holds_lwmutex(UnitId::new(99)));
    }

    #[test]
    fn allocate_child_stack_produces_non_overlapping_blocks() {
        let mut host = Lv2Host::new();
        let s1 = host.allocate_child_stack(0x10_000, 0x10).unwrap();
        let s2 = host.allocate_child_stack(0x10_000, 0x10).unwrap();
        let s3 = host.allocate_child_stack(0x10_000, 0x10).unwrap();
        assert_eq!(s1.base, 0xD010_0000);
        assert!(s2.base >= s1.end());
        assert!(s3.base >= s2.end());
    }

    #[test]
    fn is_ppu_thread_finished_for_unit_tracks_thread_state() {
        use crate::ppu_thread::{PpuThreadAttrs, PpuThreadState};
        let mut host = Lv2Host::new();
        let parent = UnitId::new(0);
        assert!(!host.is_ppu_thread_finished_for_unit(parent));

        host.seed_primary_ppu_thread(
            parent,
            PpuThreadAttrs {
                entry: 0x10_0000,
                arg: 0,
                stack_base: 0xD000_0000,
                stack_size: 0x10000,
                priority: 1000,
                tls_base: 0,
            },
        );
        assert!(!host.is_ppu_thread_finished_for_unit(parent));

        let tid = host
            .ppu_threads()
            .thread_id_for_unit(parent)
            .expect("seeded primary thread has a thread id");
        host.ppu_threads_mut()
            .get_mut(tid)
            .expect("thread exists")
            .state = PpuThreadState::Finished;
        assert!(host.is_ppu_thread_finished_for_unit(parent));
    }

    #[test]
    fn fs_mounts_starts_empty() {
        let host = Lv2Host::new();
        assert_eq!(host.fs_mounts().mounts().count(), 0);
    }

    #[test]
    fn fs_mounts_mut_accepts_registration_and_resolves() {
        use std::path::PathBuf;

        let mut host = Lv2Host::new();
        let mount = crate::fs_store::FsMount::new("/app_home", PathBuf::from("/host/usr"))
            .expect("valid mount");
        host.fs_mounts_mut()
            .add(mount)
            .expect("first registration succeeds");

        let resolved = host
            .fs_mounts()
            .resolve("/app_home/Data/level.xml")
            .expect("no traversal");
        assert_eq!(
            resolved,
            Some(PathBuf::from("/host/usr").join("Data").join("level.xml"))
        );
    }

    #[test]
    fn fs_mounts_unmatched_path_returns_none() {
        use std::path::PathBuf;

        let mut host = Lv2Host::new();
        host.fs_mounts_mut()
            .add(
                crate::fs_store::FsMount::new("/dev_hdd0", PathBuf::from("/host/hdd"))
                    .expect("valid mount"),
            )
            .expect("registration succeeds");
        assert_eq!(host.fs_mounts().resolve("/app_home/foo"), Ok(None));
    }

    #[test]
    fn mmapper_alloc_first_grant_sits_at_mmapper_region_start() {
        let mut host = Lv2Host::new();
        assert_eq!(host.mmapper_alloc(0x1), Some(Lv2Host::MMAPPER_REGION_START));
    }

    #[test]
    fn mmapper_alloc_rounds_up_to_256_mib_granule() {
        let mut host = Lv2Host::new();
        let first = host.mmapper_alloc(0x1).expect("first grant");
        let second = host.mmapper_alloc(0x1).expect("second grant");
        assert_eq!(second - first, 0x1000_0000);
    }

    #[test]
    fn mmapper_alloc_cursor_advances_across_calls() {
        let mut host = Lv2Host::new();
        let a = host.mmapper_alloc(0x1).expect("first");
        let b = host.mmapper_alloc(0x1).expect("second");
        let c = host.mmapper_alloc(0x1).expect("third");
        assert_eq!(a, Lv2Host::MMAPPER_REGION_START);
        assert_eq!(b, Lv2Host::MMAPPER_REGION_START + 0x1000_0000);
        assert_eq!(c, Lv2Host::MMAPPER_REGION_START + 0x2000_0000);
    }

    #[test]
    fn mmapper_alloc_rejects_zero_size() {
        let mut host = Lv2Host::new();
        assert_eq!(host.mmapper_alloc(0), None);
        // Cursor unchanged after rejection.
        assert_eq!(host.mmapper_alloc(0x1), Some(Lv2Host::MMAPPER_REGION_START));
    }

    #[test]
    fn mmapper_alloc_caps_at_mmapper_region_end() {
        let mut host = Lv2Host::new();
        // (MMAPPER_REGION_END - MMAPPER_REGION_START) / granule
        //   = (0xC000_0000 - 0x5000_0000) / 0x1000_0000 = 7 grants.
        for _ in 0..7 {
            host.mmapper_alloc(0x1).expect("within region");
        }
        // The 8th grant would walk into the RSX dma_control MMIO
        // region at 0xC000_0000.
        assert_eq!(host.mmapper_alloc(0x1), None);
    }

    #[test]
    fn mmapper_alloc_never_returns_address_in_reserved_rsx_window() {
        // The reserved [0x4000_0000, 0x5000_0000) window holds
        // RSX_DEVICE_ADDR (and any future per-context allocations
        // placed in the same window) so mmapper handouts must
        // never alias it.
        let mut host = Lv2Host::new();
        for _ in 0..7 {
            let addr = host.mmapper_alloc(0x1).expect("within region");
            assert!(addr >= Lv2Host::MMAPPER_REGION_START);
            assert!(addr >= 0x5000_0000);
        }
    }

    #[test]
    fn alloc_id_starts_at_kernel_id_sentinel() {
        let mut host = Lv2Host::new();
        assert_eq!(host.alloc_id(), 0x4000_0001);
    }

    #[test]
    fn alloc_id_is_monotonic_across_calls() {
        let mut host = Lv2Host::new();
        let a = host.alloc_id();
        let b = host.alloc_id();
        let c = host.alloc_id();
        assert_eq!(b, a + 1);
        assert_eq!(c, b + 1);
    }

    #[test]
    fn firmware_identity_round_trip_returns_some_with_matching_digest() {
        let mut host = Lv2Host::new();
        assert!(host.firmware_identity().is_none());
        let digest: [u8; 32] = [0x5A; 32];
        host.set_firmware_identity("4.85", digest);
        let id = host
            .firmware_identity()
            .expect("identity captured after set");
        assert_eq!(id.pup_sha256_bytes, digest);
    }

    #[test]
    fn firmware_identity_image_version_hash_is_deterministic() {
        let mut a = Lv2Host::new();
        let mut b = Lv2Host::new();
        a.set_firmware_identity("4.85", [0u8; 32]);
        b.set_firmware_identity("4.85", [0u8; 32]);
        let ha = a.firmware_identity().unwrap().image_version_hash;
        let hb = b.firmware_identity().unwrap().image_version_hash;
        assert_eq!(ha, hb);
    }

    #[test]
    fn firmware_identity_distinct_versions_produce_distinct_hashes() {
        let mut a = Lv2Host::new();
        let mut b = Lv2Host::new();
        a.set_firmware_identity("4.85", [0u8; 32]);
        b.set_firmware_identity("4.86", [0u8; 32]);
        let ha = a.firmware_identity().unwrap().image_version_hash;
        let hb = b.firmware_identity().unwrap().image_version_hash;
        assert_ne!(ha, hb);
    }

    #[test]
    fn firmware_identity_set_shifts_state_hash() {
        let mut host = Lv2Host::new();
        let pre = host.state_hash();
        host.set_firmware_identity("4.85", [0u8; 32]);
        assert_ne!(pre, host.state_hash());
    }

    #[cfg(debug_assertions)]
    #[test]
    #[should_panic(expected = "firmware identity already set")]
    fn set_firmware_identity_twice_panics_in_debug() {
        let mut host = Lv2Host::new();
        host.set_firmware_identity("4.85", [0u8; 32]);
        host.set_firmware_identity("4.85", [0u8; 32]);
    }

    #[cfg(debug_assertions)]
    #[test]
    #[should_panic(expected = "mem_alloc_base must be 64 KiB aligned")]
    fn set_mem_alloc_base_rejects_misaligned_in_debug() {
        let mut host = Lv2Host::new();
        host.set_mem_alloc_base(0x0001_0001);
    }

    #[cfg(debug_assertions)]
    #[test]
    #[should_panic(expected = "PS3 user-memory floor")]
    fn set_mem_alloc_base_rejects_below_user_floor_in_debug() {
        let mut host = Lv2Host::new();
        host.set_mem_alloc_base(0);
    }

    #[cfg(debug_assertions)]
    #[test]
    #[should_panic(expected = "below SYS_RSX_MEM_BASE")]
    fn set_mem_alloc_base_rejects_inside_sys_rsx_window_in_debug() {
        let mut host = Lv2Host::new();
        host.set_mem_alloc_base(Lv2Host::SYS_RSX_MEM_BASE);
    }

    #[test]
    fn set_mem_alloc_base_accepts_aligned_base_within_user_region() {
        let mut host = Lv2Host::new();
        host.set_mem_alloc_base(0x0100_0000);
    }
}
