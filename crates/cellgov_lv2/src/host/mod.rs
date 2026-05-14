//! LV2 model the runtime calls into.
//!
//! # Cross-module contract
//!
//! The runtime calls [`Lv2Host::dispatch`] once per PPU syscall yield,
//! synchronously inside the same `step()` that observed the yield.
//! During the call the host reads guest memory through the
//! [`Lv2Runtime`] trait and returns an [`crate::dispatch::Lv2Dispatch`] telling the
//! runtime what guest-visible work to perform. The host never writes
//! guest memory directly; every write travels back to the runtime as
//! an `Effect` so the commit pipeline orders it.

pub use self::rsx::{
    SysRsxContext, PACKAGE_CELLGOV_SET_FLIP_HANDLER, PACKAGE_CELLGOV_SET_USER_HANDLER,
    PACKAGE_CELLGOV_SET_VBLANK_HANDLER,
};
use std::collections::BTreeMap;

use crate::fs_store::{FsMountTable, FsStore};
use crate::image::ContentStore;
use crate::ppu_thread::{
    PpuThread, PpuThreadAttrs, PpuThreadId, PpuThreadTable, ThreadStack, ThreadStackAllocator,
    TlsTemplate,
};
use crate::sync_primitives::{
    CondTable, EventFlagTable, EventQueueTable, LwMutexTable, MutexTable, SemaphoreTable,
};
use crate::thread_group::ThreadGroupTable;
use cellgov_event::UnitId;
use cellgov_time::GuestTicks;

// Re-exported into the cross-primitive test module via `use super::*;`.
#[cfg(test)]
#[allow(
    unused_imports,
    reason = "consumed transitively by cross_primitive_tests (#[path] include below)"
)]
use crate::{dispatch::Lv2Dispatch, request::Lv2Request};

mod callback_dispatch;
mod cond;
mod diagnostics;
mod dispatch_route;
mod event_flag;
mod event_queue;
mod fs;
mod lwmutex;
mod memory;
mod mutex;
mod ppu_thread;
mod process;
pub mod rsx;
mod runtime;
mod semaphore;
mod spu;
mod state_hash;

pub use runtime::Lv2Runtime;

pub use callback_dispatch::CallbackError;

#[cfg(test)]
mod test_support;

/// LV2 host model driven by [`Self::dispatch`].
#[derive(Debug, Clone)]
pub struct Lv2Host {
    content: ContentStore,
    groups: ThreadGroupTable,
    ppu_threads: PpuThreadTable,
    tls_template: TlsTemplate,
    stack_allocator: ThreadStackAllocator,
    /// Shared id allocator for mutex / semaphore / event-queue /
    /// event-flag / cond. `lwmutexes` has its own allocator from 1.
    next_kernel_id: u32,
    mem_alloc_ptr: u32,
    /// Separate bump cursor so the RSX-visible region cannot collide
    /// with PPU allocations.
    rsx_mem_alloc_ptr: u32,
    rsx_mem_handle_counter: u32,
    rsx_context: SysRsxContext,
    lwmutexes: LwMutexTable,
    mutexes: MutexTable,
    semaphores: SemaphoreTable,
    event_queues: EventQueueTable,
    event_flags: EventFlagTable,
    conds: CondTable,
    /// Dispatch-local scratch; not folded into [`Self::state_hash`].
    current_tick: GuestTicks,
    /// Running count of host-invariant breaks. Non-zero means at
    /// least one wake or table update fell back to a degraded
    /// response. Not hashed.
    invariant_break_count: usize,
    /// Captured `sys_tty_write` byte stream in dispatch order.
    /// Observation channel; not folded into [`Self::state_hash`].
    tty_log: Vec<u8>,
    /// Live-object counters for primitives stubbed as ID allocators
    /// only; feed `sys_process_get_number_of_object`. Not folded
    /// into [`Self::state_hash`] -- counts are derived helpers,
    /// not primary state; they only move when something else
    /// in this struct also moves.
    process_counts: process::ProcessCounts,
    fs_store: FsStore,
    /// Consulted by the dispatch layer when a guest path is not
    /// pre-registered in [`Self::fs_store`]. Boot populates from the
    /// title manifest; immutable thereafter.
    fs_mounts: FsMountTable,
    /// Per-thread count of distinct lwmutexes held. Recursive
    /// re-acquires of the same lwmutex do not bump the count; only
    /// first-acquire (FREE -> me) and kernel-side transfer
    /// (LwMutexWake) do. Read by the runtime to drive
    /// critical-section-aware scheduler stickiness.
    lwmutex_holds: BTreeMap<PpuThreadId, u32>,
    /// Worker -> (parent unit, stage) linkage for callback-dispatch
    /// workers spawned via [`Self::call_guest_callback_sync`].
    ///
    /// # Cross-module contract
    /// The parent unit stays parked while its entry is present.
    /// `dispatch_callback_return` consumes the entry to build the
    /// wake response; if the worker terminates without producing a
    /// return event, the entry leaks and the parent never wakes.
    /// `BTreeMap` for deterministic iteration.
    callback_parents: BTreeMap<PpuThreadId, (UnitId, crate::dispatch::CallbackReturnStage)>,
    /// Per-parent recursion depth, capped at
    /// `cellgov_ps3_abi::callback_dispatch::CALLBACK_DEPTH_CAP`.
    callback_depth: BTreeMap<UnitId, u8>,
    /// Firmware identity from `firmware.toml`. `None` until the CLI
    /// boot path verifies the manifest; folded into [`Self::state_hash`]
    /// when set so two boots of the same install hash identically.
    firmware_identity: Option<FirmwareIdentity>,
}

/// Captured at boot via the verified `firmware.toml` manifest. The
/// `image_version` hash and `pup_sha256` together identify the PUP
/// the install came from; both fold into `Lv2Host::state_hash`.
#[derive(Debug, Clone)]
pub struct FirmwareIdentity {
    pub image_version_hash: u64,
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
            rsx_mem_alloc_ptr: Self::SYS_RSX_MEM_BASE,
            rsx_mem_handle_counter: 1,
            rsx_context: SysRsxContext::new(),
            lwmutexes: LwMutexTable::new(),
            mutexes: MutexTable::new(),
            semaphores: SemaphoreTable::new(),
            event_queues: EventQueueTable::new(),
            event_flags: EventFlagTable::new(),
            conds: CondTable::new(),
            current_tick: GuestTicks::ZERO,
            invariant_break_count: 0,
            tty_log: Vec::new(),
            process_counts: process::ProcessCounts::new(),
            fs_store,
            fs_mounts: FsMountTable::new(),
            lwmutex_holds: BTreeMap::new(),
            callback_parents: BTreeMap::new(),
            callback_depth: BTreeMap::new(),
            firmware_identity: None,
        }
    }

    /// Record the verified-firmware identity from the CLI boot path.
    /// `image_version` is FNV-1a-hashed; `pup_sha256_bytes` is the
    /// 32-byte SHA-256 digest. Both fold into [`Self::state_hash`]
    /// so two boots of the same install produce identical hashes.
    pub fn set_firmware_identity(&mut self, image_version: &str, pup_sha256_bytes: [u8; 32]) {
        let mut h = cellgov_mem::Fnv1aHasher::new();
        h.write(image_version.as_bytes());
        self.firmware_identity = Some(FirmwareIdentity {
            image_version_hash: h.finish(),
            pup_sha256_bytes,
        });
    }

    /// Read-only view of the captured firmware identity.
    pub fn firmware_identity(&self) -> Option<&FirmwareIdentity> {
        self.firmware_identity.as_ref()
    }

    /// Read-only view of the in-memory filesystem store.
    pub fn fs_store(&self) -> &FsStore {
        &self.fs_store
    }

    /// Mutable view of the in-memory filesystem store.
    pub fn fs_store_mut(&mut self) -> &mut FsStore {
        &mut self.fs_store
    }

    /// Read-only view of the mount table.
    pub fn fs_mounts(&self) -> &FsMountTable {
        &self.fs_mounts
    }

    /// Boot wires real mounts here; the dispatch layer treats the
    /// table as read-only.
    pub fn fs_mounts_mut(&mut self) -> &mut FsMountTable {
        &mut self.fs_mounts
    }

    /// Distinct lwmutexes currently held by `tid`.
    pub fn lwmutex_holds_for(&self, tid: PpuThreadId) -> u32 {
        self.lwmutex_holds.get(&tid).copied().unwrap_or(0)
    }

    /// # Contract
    /// Bumps the count for a first-acquire (FREE -> tid) or a
    /// kernel-side transfer. Recursive re-acquires (tid already
    /// the owner) are tracked elsewhere and must not pass through
    /// this entry.
    pub fn lwmutex_holds_inc(&mut self, tid: PpuThreadId) {
        let slot = self.lwmutex_holds.entry(tid).or_insert(0);
        *slot = slot.saturating_add(1);
    }

    /// Saturates at 0; underflow signals a leak in the increment
    /// path.
    pub fn lwmutex_holds_dec(&mut self, tid: PpuThreadId) {
        if let Some(slot) = self.lwmutex_holds.get_mut(&tid) {
            *slot = slot.saturating_sub(1);
            if *slot == 0 {
                self.lwmutex_holds.remove(&tid);
            }
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
        self.mem_alloc_ptr = base;
    }

    /// Read-only view of the sys_rsx host context.
    #[inline]
    pub fn sys_rsx_context(&self) -> &SysRsxContext {
        &self.rsx_context
    }

    pub(super) fn alloc_id(&mut self) -> u32 {
        let id = self.next_kernel_id;
        self.next_kernel_id += 1;
        id
    }

    /// Read-only view of the per-title content manifest store.
    pub fn content_store(&self) -> &ContentStore {
        &self.content
    }

    /// Mutable view of the per-title content manifest store.
    pub fn content_store_mut(&mut self) -> &mut ContentStore {
        &mut self.content
    }

    /// Read-only view of the SPU thread-group table.
    pub fn thread_groups(&self) -> &ThreadGroupTable {
        &self.groups
    }

    /// Mutable view of the SPU thread-group table.
    pub fn thread_groups_mut(&mut self) -> &mut ThreadGroupTable {
        &mut self.groups
    }

    /// Read-only view of the PPU thread table.
    pub fn ppu_threads(&self) -> &PpuThreadTable {
        &self.ppu_threads
    }

    /// Mutable view of the PPU thread table.
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

    /// True when the unit's backing `PpuThread` is `Finished`. The
    /// runtime mirrors a host-driven thread finish (callback-worker
    /// terminal trampoline) into the unit's lifecycle so the PPU
    /// execution loop stops fetching past the trampoline `sc 0`.
    /// `false` when the unit is not a PPU thread or the thread is
    /// still `Running` / `Detached`.
    pub fn is_ppu_thread_finished_for_unit(&self, unit_id: UnitId) -> bool {
        match self.ppu_threads.get_by_unit(unit_id) {
            Some(thread) => matches!(thread.state, crate::ppu_thread::PpuThreadState::Finished),
            None => false,
        }
    }

    /// Install the TLS template used for new PPU threads.
    pub fn set_tls_template(&mut self, template: TlsTemplate) {
        self.tls_template = template;
    }

    /// Read-only view of the installed TLS template.
    pub fn tls_template(&self) -> &TlsTemplate {
        &self.tls_template
    }

    /// Read-only view of the lwmutex table.
    pub fn lwmutexes(&self) -> &LwMutexTable {
        &self.lwmutexes
    }

    /// Mutable view of the lwmutex table.
    pub fn lwmutexes_mut(&mut self) -> &mut LwMutexTable {
        &mut self.lwmutexes
    }

    /// Read-only view of the mutex table.
    pub fn mutexes(&self) -> &MutexTable {
        &self.mutexes
    }

    /// Mutable view of the mutex table.
    pub fn mutexes_mut(&mut self) -> &mut MutexTable {
        &mut self.mutexes
    }

    /// Read-only view of the semaphore table.
    pub fn semaphores(&self) -> &SemaphoreTable {
        &self.semaphores
    }

    /// Mutable view of the semaphore table.
    pub fn semaphores_mut(&mut self) -> &mut SemaphoreTable {
        &mut self.semaphores
    }

    /// Read-only view of the event-queue table.
    pub fn event_queues(&self) -> &EventQueueTable {
        &self.event_queues
    }

    /// Mutable view of the event-queue table.
    pub fn event_queues_mut(&mut self) -> &mut EventQueueTable {
        &mut self.event_queues
    }

    /// Read-only view of the event-flag table.
    pub fn event_flags(&self) -> &EventFlagTable {
        &self.event_flags
    }

    /// Read-only view of the condition-variable table.
    pub fn conds(&self) -> &CondTable {
        &self.conds
    }

    /// Mutable view of the condition-variable table.
    pub fn conds_mut(&mut self) -> &mut CondTable {
        &mut self.conds
    }

    /// Mutable view of the event-flag table.
    pub fn event_flags_mut(&mut self) -> &mut EventFlagTable {
        &mut self.event_flags
    }

    /// Allocate a new child-thread stack of `size` bytes at `align`.
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
#[path = "../tests/host_tests.rs"]
mod cross_primitive_tests;

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
        assert_eq!(s1.base, 0xD001_0000);
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
}
