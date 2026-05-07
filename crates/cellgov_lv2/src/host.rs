//! LV2 model the runtime calls into.
//!
//! # Cross-module contract
//!
//! The runtime calls [`Lv2Host::dispatch`] once per PPU syscall yield,
//! synchronously inside the same `step()` that observed the yield.
//! During the call the host reads guest memory through the
//! [`Lv2Runtime`] trait and returns an [`Lv2Dispatch`] telling the
//! runtime what guest-visible work to perform. The host never writes
//! guest memory directly; every write travels back to the runtime as
//! an `Effect` so the commit pipeline orders it.

pub use self::rsx::{
    SysRsxContext, PACKAGE_CELLGOV_SET_FLIP_HANDLER, PACKAGE_CELLGOV_SET_USER_HANDLER,
    PACKAGE_CELLGOV_SET_VBLANK_HANDLER,
};
use std::collections::BTreeMap;

use crate::dispatch::Lv2Dispatch;
use crate::fs::FsStore;
use crate::image::ContentStore;
use crate::ppu_thread::{
    PpuThread, PpuThreadAttrs, PpuThreadId, PpuThreadTable, ThreadStack, ThreadStackAllocator,
    TlsTemplate,
};
use crate::request::Lv2Request;
use crate::sync_primitives::{
    CondTable, EventFlagTable, EventQueueTable, LwMutexTable, MutexTable, SemaphoreTable,
};
use crate::thread_group::ThreadGroupTable;
use cellgov_effects::{Effect, WritePayload};
use cellgov_event::{PriorityClass, UnitId};
use cellgov_mem::{ByteRange, GuestAddr};
use cellgov_ps3_abi::cell_errors as errno;
use cellgov_time::GuestTicks;

/// Readonly view of runtime state exposed to the host during dispatch.
///
/// `current_tick` stamps LV2-sourced effects so they participate in
/// commit-pipeline ordering at the triggering syscall's tick rather
/// than tick 0.
pub trait Lv2Runtime {
    /// # Contract
    /// `Some(bytes)` must carry exactly `len` bytes; short reads are
    /// a trait violation. `None` means the range is out of bounds.
    fn read_committed(&self, addr: u64, len: usize) -> Option<&[u8]>;

    /// Current guest tick.
    fn current_tick(&self) -> GuestTicks;

    /// Read up to `max_len` bytes from `addr`, returning the prefix
    /// before the first `terminator` byte (terminator excluded).
    ///
    /// # Returns
    /// - `Some(bytes)` with `bytes.len() < max_len` when a terminator
    ///   is found within the first `max_len` mapped bytes.
    /// - `None` when `addr` is unmapped, no terminator appears within
    ///   `max_len` mapped bytes, or the address is in a
    ///   `ReservedStrict` region.
    fn read_committed_until(&self, addr: u64, max_len: usize, terminator: u8) -> Option<&[u8]>;

    /// True iff a `len`-byte write at `addr` lands entirely inside a
    /// single `ReadWrite` region.
    fn writable(&self, addr: u64, len: usize) -> bool;
}

mod callback_dispatch;
mod cond;
mod event_flag;
mod event_queue;
mod fs;
mod lwmutex;
mod memory;
mod mutex;
mod ppu_thread;
pub mod rsx;
mod semaphore;
mod spu;

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
    /// only; feed `sys_process_get_number_of_object`.
    timer_count: u32,
    rwlock_count: u32,
    event_port_count: u32,
    lwcond_count: u32,
    /// Live count of file descriptors opened via `sys_fs_open`;
    /// feeds the `SYS_FS_FD_OBJECT` (0x73) query.
    fs_fd_count: u32,
    fs_store: FsStore,
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
    pub fn new() -> Self {
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
            timer_count: 0,
            rwlock_count: 0,
            event_port_count: 0,
            lwcond_count: 0,
            fs_fd_count: 0,
            fs_store: FsStore::new(),
            lwmutex_holds: BTreeMap::new(),
            callback_parents: BTreeMap::new(),
            callback_depth: BTreeMap::new(),
        }
    }

    /// Read-only view of the in-memory filesystem store.
    pub fn fs_store(&self) -> &FsStore {
        &self.fs_store
    }

    /// Mutable view of the in-memory filesystem store.
    pub fn fs_store_mut(&mut self) -> &mut FsStore {
        &mut self.fs_store
    }

    /// Distinct lwmutexes currently held by `tid`.
    pub fn lwmutex_holds_for(&self, tid: PpuThreadId) -> u32 {
        self.lwmutex_holds.get(&tid).copied().unwrap_or(0)
    }

    /// # Contract
    /// Recursive re-acquires (where `tid` was already the owner)
    /// must not call this; only first-acquire (FREE -> me) and
    /// kernel-side transfer paths bump the count.
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

    /// No decrement counterpart: real PS3's `sys_fs_close` does not
    /// drop the kernel-side fs-object count synchronously, and the
    /// ps3autotests `sys_process` matrix shows `fs_fd` staying at 1
    /// after `fclose`.
    pub(super) fn fs_fd_count_inc(&mut self) {
        self.fs_fd_count = self.fs_fd_count.saturating_add(1);
    }

    /// Increment the live `sys_lwcond` object count.
    pub fn lwcond_count_inc(&mut self) {
        self.lwcond_count = self.lwcond_count.saturating_add(1);
    }

    /// Decrement the live `sys_lwcond` object count, saturating at 0.
    pub fn lwcond_count_dec(&mut self) {
        self.lwcond_count = self.lwcond_count.saturating_sub(1);
    }

    /// Captured `sys_tty_write` byte stream in dispatch order.
    #[inline]
    pub fn tty_log(&self) -> &[u8] {
        &self.tty_log
    }

    /// Running count of host-invariant breaks observed during dispatch.
    #[inline]
    pub fn invariant_break_count(&self) -> usize {
        self.invariant_break_count
    }

    /// Debug-panic + log-once for a host-invariant break.
    pub(super) fn record_invariant_break(
        &mut self,
        site: &'static str,
        details: std::fmt::Arguments<'_>,
    ) {
        debug_assert!(false, "lv2 host invariant break at {site}: {details}");
        self.log_invariant_break(site, details);
    }

    /// Log-once without `debug_assert!`, for paths reachable by
    /// guest input during normal operation (e.g. `Unsupported`
    /// syscalls during real boots).
    fn log_invariant_break(&mut self, site: &'static str, details: std::fmt::Arguments<'_>) {
        if self.invariant_break_count == 0 {
            eprintln!("lv2 host invariant break at {site}: {details}");
        }
        self.invariant_break_count = self.invariant_break_count.saturating_add(1);
    }

    /// `None` means the thread table and the primitive diverged;
    /// the divergence is logged as an invariant break so the caller
    /// can skip the wake and leave surviving waiters intact.
    pub(super) fn resolve_wake_thread(
        &mut self,
        thread: PpuThreadId,
        site: &'static str,
    ) -> Option<UnitId> {
        match self.ppu_threads.get(thread) {
            Some(t) => Some(t.unit_id),
            None => {
                self.record_invariant_break(
                    site,
                    format_args!(
                        "PpuThreadId {thread:?} dequeued from a primitive waiter list but \
                         not in PpuThreadTable; wake skipped"
                    ),
                );
                None
            }
        }
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

    /// FNV-1a of all committed LV2 host state; folded into the
    /// runtime's `sync_state_hash` at every commit boundary.
    ///
    /// # Gating
    /// Per-primitive tables and the child-stack allocator contribute
    /// only when non-empty / past their sentinel. `next_kernel_id`
    /// and `mem_alloc_ptr` always contribute, so a
    /// created-then-destroyed primitive still advances the hash via
    /// allocator state once the table empties again.
    ///
    /// # Cost
    /// Linear in the number of live primitives plus per-thread
    /// lwmutex-hold and callback-parent map sizes; runs once per
    /// commit boundary.
    pub fn state_hash(&self) -> u64 {
        let mut hasher = cellgov_mem::Fnv1aHasher::new();
        for source in [self.content.state_hash(), self.groups.state_hash()] {
            hasher.write(&source.to_le_bytes());
        }
        hasher.write(&self.next_kernel_id.to_le_bytes());
        hasher.write(&self.mem_alloc_ptr.to_le_bytes());
        hasher.write(&self.rsx_mem_alloc_ptr.to_le_bytes());
        hasher.write(&self.rsx_mem_handle_counter.to_le_bytes());
        hasher.write(&self.rsx_context.state_hash().to_le_bytes());
        if !self.ppu_threads.is_empty() {
            hasher.write(&self.ppu_threads.state_hash().to_le_bytes());
        }
        if !self.tls_template.is_empty() {
            hasher.write(&self.tls_template.state_hash().to_le_bytes());
        }
        if let Some(peek) = self.stack_allocator.peek_next(0x10) {
            if peek != ThreadStackAllocator::CHILD_STACK_BASE {
                hasher.write(&peek.to_le_bytes());
            }
        }
        if !self.lwmutexes.is_empty() {
            hasher.write(&self.lwmutexes.state_hash().to_le_bytes());
        }
        if !self.mutexes.is_empty() {
            hasher.write(&self.mutexes.state_hash().to_le_bytes());
        }
        if !self.semaphores.is_empty() {
            hasher.write(&self.semaphores.state_hash().to_le_bytes());
        }
        if !self.event_queues.is_empty() {
            hasher.write(&self.event_queues.state_hash().to_le_bytes());
        }
        if !self.event_flags.is_empty() {
            hasher.write(&self.event_flags.state_hash().to_le_bytes());
        }
        if !self.conds.is_empty() {
            hasher.write(&self.conds.state_hash().to_le_bytes());
        }
        if !self.lwmutex_holds.is_empty() {
            hasher.write(&(self.lwmutex_holds.len() as u64).to_le_bytes());
            for (tid, count) in &self.lwmutex_holds {
                hasher.write(&tid.raw().to_le_bytes());
                hasher.write(&count.to_le_bytes());
            }
        }
        if !self.fs_store.is_empty() {
            hasher.write(&self.fs_store.state_hash().to_le_bytes());
        }
        if !self.callback_parents.is_empty() {
            hasher.write(&(self.callback_parents.len() as u64).to_le_bytes());
            for (worker, (parent, stage)) in &self.callback_parents {
                hasher.write(&worker.raw().to_le_bytes());
                hasher.write(&parent.raw().to_le_bytes());
                hasher.write(&[stage.stable_tag()]);
            }
        }
        if !self.callback_depth.is_empty() {
            hasher.write(&(self.callback_depth.len() as u64).to_le_bytes());
            for (parent, depth) in &self.callback_depth {
                hasher.write(&parent.raw().to_le_bytes());
                hasher.write(&[*depth]);
            }
        }
        hasher.finish()
    }

    /// Dispatch one syscall request.
    ///
    /// # Cross-module contract
    /// Called once per PPU syscall yield, synchronously inside the
    /// runtime's `step()`. The returned [`Lv2Dispatch`] is the
    /// host's complete response: any guest-memory writes ride as
    /// `Effect`s the runtime feeds into the commit pipeline. The
    /// host snapshots `rt.current_tick()` on entry so every effect
    /// it builds is stamped at the triggering syscall's tick rather
    /// than tick 0.
    pub fn dispatch(
        &mut self,
        request: Lv2Request,
        requester: UnitId,
        rt: &dyn Lv2Runtime,
    ) -> Lv2Dispatch {
        self.current_tick = rt.current_tick();
        match request {
            Lv2Request::SpuImageOpen { img_ptr, path_ptr } => {
                self.dispatch_image_open(img_ptr, path_ptr, requester, rt)
            }
            Lv2Request::SpuThreadGroupCreate {
                id_ptr,
                num_threads,
                ..
            } => self.dispatch_group_create(id_ptr, num_threads, requester),
            req @ Lv2Request::SpuThreadInitialize { .. } => {
                self.dispatch_thread_initialize(req, requester, rt)
            }
            Lv2Request::SpuThreadGroupStart { group_id } => self.dispatch_group_start(group_id),
            Lv2Request::SpuThreadGroupJoin {
                group_id,
                cause_ptr,
                status_ptr,
            } => self.dispatch_group_join(group_id, cause_ptr, status_ptr, requester),
            Lv2Request::SpuThreadGroupTerminate { group_id, value } => {
                self.log_invariant_break(
                    "dispatch.spu_thread_group_terminate_stub",
                    format_args!(
                        "sys_spu_thread_group_terminate(group_id={group_id}, value={value}) \
                         stubbed; no SPU teardown performed"
                    ),
                );
                Lv2Dispatch::Immediate {
                    code: 0,
                    effects: vec![],
                }
            }
            Lv2Request::SpuThreadWriteMb { thread_id, value } => {
                self.dispatch_write_mb(thread_id, value, requester)
            }
            Lv2Request::TtyWrite {
                buf_ptr,
                len,
                nwritten_ptr,
                ..
            } => self.dispatch_tty_write(buf_ptr, len, nwritten_ptr, requester, rt),
            Lv2Request::LwMutexCreate { id_ptr, .. } => {
                self.dispatch_lwmutex_create(id_ptr, requester)
            }
            Lv2Request::LwMutexDestroy { id } => self.dispatch_lwmutex_destroy(id),
            Lv2Request::LwMutexLock { id, mutex_ptr, .. } => {
                self.dispatch_lwmutex_lock(id, mutex_ptr, requester)
            }
            Lv2Request::LwMutexUnlock { id } => self.dispatch_lwmutex_unlock(id, requester),
            Lv2Request::LwMutexTryLock { id } => self.dispatch_lwmutex_trylock(id, requester),
            Lv2Request::FsOpen {
                path_ptr,
                flags,
                fd_out_ptr,
                mode,
            } => self.dispatch_fs_open(path_ptr, flags, fd_out_ptr, mode, requester, rt),
            Lv2Request::FsClose { fd } => self.dispatch_fs_close(fd),
            Lv2Request::FsRead {
                fd,
                buf_ptr,
                nbytes,
                nread_out_ptr,
            } => self.dispatch_fs_read(fd, buf_ptr, nbytes, nread_out_ptr, requester, rt),
            Lv2Request::FsLseek {
                fd,
                offset,
                whence,
                pos_out_ptr,
            } => self.dispatch_fs_lseek(fd, offset, whence, pos_out_ptr, requester, rt),
            Lv2Request::FsFstat { fd, stat_out_ptr } => {
                self.dispatch_fs_fstat(fd, stat_out_ptr, requester, rt)
            }
            Lv2Request::FsStat {
                path_ptr,
                stat_out_ptr,
            } => self.dispatch_fs_stat(path_ptr, stat_out_ptr, requester, rt),
            Lv2Request::FsWrite {
                buf_ptr,
                size,
                nwrite_ptr,
                ..
            } => self.dispatch_tty_write(buf_ptr, size, nwrite_ptr, requester, rt),
            Lv2Request::MutexCreate { id_ptr, attr_ptr } => {
                self.dispatch_mutex_create(id_ptr, attr_ptr, requester, rt)
            }
            Lv2Request::MutexDestroy { mutex_id } => self.dispatch_mutex_destroy(mutex_id),
            Lv2Request::MutexLock { mutex_id, .. } => self.dispatch_mutex_lock(mutex_id, requester),
            Lv2Request::MutexUnlock { mutex_id } => self.dispatch_mutex_unlock(mutex_id, requester),
            Lv2Request::MutexTryLock { mutex_id } => {
                self.dispatch_mutex_trylock(mutex_id, requester)
            }
            Lv2Request::SemaphoreCreate {
                id_ptr,
                attr_ptr,
                initial,
                max,
            } => self.dispatch_semaphore_create(id_ptr, attr_ptr, initial, max, requester, rt),
            Lv2Request::SemaphoreDestroy { id } => self.dispatch_semaphore_destroy(id),
            Lv2Request::SemaphoreWait { id, timeout } => {
                self.dispatch_semaphore_wait(id, timeout, requester)
            }
            Lv2Request::SemaphorePost { id, val } => self.dispatch_semaphore_post(id, val),
            Lv2Request::SemaphoreTryWait { id } => self.dispatch_semaphore_trywait(id),
            Lv2Request::SemaphoreGetValue { id, out_ptr } => {
                self.dispatch_semaphore_get_value(id, out_ptr, requester)
            }
            Lv2Request::EventQueueCreate { id_ptr, size, .. } => {
                self.dispatch_event_queue_create(id_ptr, size, requester)
            }
            Lv2Request::EventQueueDestroy { queue_id } => {
                self.dispatch_event_queue_destroy(queue_id)
            }
            Lv2Request::EventQueueReceive {
                queue_id, out_ptr, ..
            } => self.dispatch_event_queue_receive(queue_id, out_ptr, requester),
            Lv2Request::EventPortSend {
                port_id,
                data1,
                data2,
                data3,
            } => self.dispatch_event_port_send(port_id, data1, data2, data3),
            Lv2Request::EventQueueTryReceive {
                queue_id,
                event_array,
                size,
                count_out,
            } => self.dispatch_event_queue_tryreceive(
                queue_id,
                event_array,
                size,
                count_out,
                requester,
            ),
            Lv2Request::EventFlagCreate {
                id_ptr,
                attr_ptr,
                init,
            } => self.dispatch_event_flag_create(id_ptr, attr_ptr, init, requester, rt),
            Lv2Request::EventFlagDestroy { id } => self.dispatch_event_flag_destroy(id),
            Lv2Request::EventFlagWait {
                id,
                bits,
                mode,
                result_ptr,
                timeout,
            } => self.dispatch_event_flag_wait(id, bits, mode, result_ptr, timeout, requester),
            Lv2Request::EventFlagTryWait {
                id,
                bits,
                mode,
                result_ptr,
            } => self.dispatch_event_flag_trywait(id, bits, mode, result_ptr, requester),
            Lv2Request::EventFlagSet { id, bits } => self.dispatch_event_flag_set(id, bits),
            Lv2Request::EventFlagClear { id, bits } => self.dispatch_event_flag_clear(id, bits),
            Lv2Request::EventFlagCancel { id, num_ptr } => {
                self.dispatch_event_flag_cancel(id, num_ptr, requester)
            }
            Lv2Request::EventFlagGet { id, flags_ptr } => {
                self.dispatch_event_flag_get(id, flags_ptr, requester)
            }
            Lv2Request::CondCreate {
                id_ptr, mutex_id, ..
            } => self.dispatch_cond_create(id_ptr, mutex_id, requester),
            Lv2Request::CondDestroy { id } => self.dispatch_cond_destroy(id),
            Lv2Request::CondWait { id, .. } => self.dispatch_cond_wait(id, requester),
            Lv2Request::CondSignal { id } => self.dispatch_cond_signal(id),
            Lv2Request::CondSignalAll { id } => self.dispatch_cond_signal_all(id),
            Lv2Request::CondSignalTo { id, target_thread } => {
                self.dispatch_cond_signal_to(id, target_thread)
            }
            Lv2Request::MemoryAllocate {
                size,
                alloc_addr_ptr,
                ..
            } => self.dispatch_memory_allocate(size, alloc_addr_ptr, requester),
            Lv2Request::MemoryFree { .. } => {
                // No dealloc tracking; titles keying on free's
                // errno will misbehave.
                Lv2Dispatch::Immediate {
                    code: 0u64,
                    effects: vec![],
                }
            }
            Lv2Request::MemoryContainerCreate { cid_ptr, .. } => {
                let id = self.alloc_id();
                self.immediate_write_u32(id, cid_ptr, requester)
            }
            // The round-robin walk advances on the syscall yield
            // itself, so the host has nothing further to do.
            Lv2Request::PpuThreadYield => Lv2Dispatch::Immediate {
                code: 0,
                effects: vec![],
            },
            Lv2Request::TimeGetTimebaseFrequency => Lv2Dispatch::Immediate {
                code: cellgov_time::CELL_PPU_TIMEBASE_HZ,
                effects: vec![],
            },
            Lv2Request::TimeGetTimezone {
                timezone_ptr,
                summer_time_ptr,
            } => {
                let zero = 0i32.to_be_bytes();
                let tz_write = Effect::SharedWriteIntent {
                    range: ByteRange::new(GuestAddr::new(timezone_ptr as u64), 4).unwrap(),
                    bytes: WritePayload::from_slice(&zero),
                    ordering: PriorityClass::Normal,
                    source: requester,
                    source_time: self.current_tick,
                };
                let dst_write = Effect::SharedWriteIntent {
                    range: ByteRange::new(GuestAddr::new(summer_time_ptr as u64), 4).unwrap(),
                    bytes: WritePayload::from_slice(&zero),
                    ordering: PriorityClass::Normal,
                    source: requester,
                    source_time: self.current_tick,
                };
                Lv2Dispatch::Immediate {
                    code: 0,
                    effects: vec![tz_write, dst_write],
                }
            }
            Lv2Request::MemoryGetUserMemorySize { mem_info_ptr } => {
                let total = cellgov_ps3_abi::sys_memory::USER_MEMORY_TOTAL;
                let available = total;
                let mut bytes = [0u8; 8];
                bytes[0..4].copy_from_slice(&total.to_be_bytes());
                bytes[4..8].copy_from_slice(&available.to_be_bytes());
                let write = Effect::SharedWriteIntent {
                    range: ByteRange::new(GuestAddr::new(mem_info_ptr as u64), 8).unwrap(),
                    bytes: WritePayload::from_slice(&bytes),
                    ordering: PriorityClass::Normal,
                    source: requester,
                    source_time: self.current_tick,
                };
                Lv2Dispatch::Immediate {
                    code: 0,
                    effects: vec![write],
                }
            }
            Lv2Request::TimeGetCurrentTime { sec_ptr, nsec_ptr } => {
                let (sec, nsec) = cellgov_time::ticks_to_sec_nsec(rt.current_tick().raw());
                let sec_write = Effect::SharedWriteIntent {
                    range: ByteRange::new(GuestAddr::new(sec_ptr as u64), 8).unwrap(),
                    bytes: WritePayload::from_slice(&sec.to_be_bytes()),
                    ordering: PriorityClass::Normal,
                    source: requester,
                    source_time: self.current_tick,
                };
                let nsec_write = Effect::SharedWriteIntent {
                    range: ByteRange::new(GuestAddr::new(nsec_ptr as u64), 8).unwrap(),
                    bytes: WritePayload::from_slice(&nsec.to_be_bytes()),
                    ordering: PriorityClass::Normal,
                    source: requester,
                    source_time: self.current_tick,
                };
                Lv2Dispatch::Immediate {
                    code: 0,
                    effects: vec![sec_write, nsec_write],
                }
            }
            Lv2Request::PpuThreadExit { exit_value } => {
                self.dispatch_ppu_thread_exit(exit_value, requester)
            }
            Lv2Request::PpuThreadCreate {
                id_ptr,
                entry_opd,
                arg,
                priority,
                stacksize,
                flags: _,
            } => self.dispatch_ppu_thread_create(id_ptr, entry_opd, arg, priority, stacksize, rt),
            Lv2Request::PpuThreadJoin {
                target,
                status_out_ptr,
            } => self.dispatch_ppu_thread_join(target, status_out_ptr, requester),
            Lv2Request::SysRsxMemoryAllocate {
                mem_handle_ptr,
                mem_addr_ptr,
                size,
                ..
            } => {
                self.dispatch_sys_rsx_memory_allocate(mem_handle_ptr, mem_addr_ptr, size, requester)
            }
            Lv2Request::SysRsxMemoryFree { .. } => self.dispatch_sys_rsx_memory_free_noop(),
            Lv2Request::SysRsxContextAllocate {
                context_id_ptr,
                lpar_dma_control_ptr,
                lpar_driver_info_ptr,
                lpar_reports_ptr,
                mem_ctx,
                system_mode,
            } => self.dispatch_sys_rsx_context_allocate(
                context_id_ptr,
                lpar_dma_control_ptr,
                lpar_driver_info_ptr,
                lpar_reports_ptr,
                mem_ctx,
                system_mode,
                requester,
            ),
            Lv2Request::SysRsxContextFree { .. } => self.dispatch_sys_rsx_context_free_noop(),
            Lv2Request::SysRsxContextAttribute {
                context_id,
                package_id,
                a3,
                a4,
                a5,
                a6,
            } => self.dispatch_sys_rsx_context_attribute(context_id, package_id, a3, a4, a5, a6),
            // _sys_prx_start_module: liblv2 calls this with id=0
            // (our _sys_prx_load_module stub returns 0); CELL_OK
            // leaves it reading uninitialized stack. Real LV2
            // returns EINVAL for id=0/null pOpt.
            Lv2Request::Unsupported { number: 481, .. } => Lv2Dispatch::Immediate {
                code: errno::CELL_EINVAL.into(),
                effects: vec![],
            },
            // sys_tty_read: CELL_OK spins CRT input loops forever;
            // real LV2 returns EIO outside debug console mode.
            Lv2Request::Unsupported { number: 402, .. } => Lv2Dispatch::Immediate {
                code: errno::CELL_EIO.into(),
                effects: vec![],
            },
            Lv2Request::ProcessExit { .. } => Lv2Dispatch::Immediate {
                code: 0u64,
                effects: vec![],
            },
            // CellGov hosts a single synthetic process; PSL1GHT
            // tests rely on these spec PID/PPID/GUID constants.
            Lv2Request::ProcessGetPid => Lv2Dispatch::Immediate {
                code: 0x0100_0500,
                effects: vec![],
            },
            Lv2Request::ProcessGetPpid => Lv2Dispatch::Immediate {
                code: 0x0100_0300,
                effects: vec![],
            },
            Lv2Request::ProcessGetPpuGuid => Lv2Dispatch::Immediate {
                code: 0x0100_0300,
                effects: vec![],
            },
            Lv2Request::ProcessIsStack { .. } => Lv2Dispatch::Immediate {
                code: 0u64,
                effects: vec![],
            },
            Lv2Request::ProcessGetNumberOfObject {
                class_id,
                count_out_ptr,
            } => self.dispatch_process_get_number_of_object(class_id, count_out_ptr, requester),
            Lv2Request::ProcessGetSdkVersion {
                version_out_ptr, ..
            } => self.dispatch_process_get_sdk_version(version_out_ptr, requester),
            Lv2Request::ProcessGetParamsfo { buf_ptr } => {
                self.dispatch_process_get_paramsfo(buf_ptr, requester)
            }
            // ID-allocator stubs for primitives whose only test-level
            // exercise is create/destroy plus the live-count probe.
            Lv2Request::TimerCreate { id_ptr } => {
                self.timer_count = self.timer_count.saturating_add(1);
                let id = self.alloc_id();
                self.immediate_write_u32(id, id_ptr, requester)
            }
            Lv2Request::TimerDestroy { .. } => {
                self.timer_count = self.timer_count.saturating_sub(1);
                Lv2Dispatch::Immediate {
                    code: 0,
                    effects: vec![],
                }
            }
            Lv2Request::RwlockCreate { id_ptr, .. } => {
                self.rwlock_count = self.rwlock_count.saturating_add(1);
                let id = self.alloc_id();
                self.immediate_write_u32(id, id_ptr, requester)
            }
            Lv2Request::RwlockDestroy { .. } => {
                self.rwlock_count = self.rwlock_count.saturating_sub(1);
                Lv2Dispatch::Immediate {
                    code: 0,
                    effects: vec![],
                }
            }
            Lv2Request::EventPortCreate { id_ptr, .. } => {
                self.event_port_count = self.event_port_count.saturating_add(1);
                let id = self.alloc_id();
                self.immediate_write_u32(id, id_ptr, requester)
            }
            Lv2Request::EventPortDestroy { .. } => {
                self.event_port_count = self.event_port_count.saturating_sub(1);
                Lv2Dispatch::Immediate {
                    code: 0,
                    effects: vec![],
                }
            }
            Lv2Request::CallbackDispatchSpawn { .. } => {
                // Fabricated internally via `call_guest_callback_sync`;
                // reaching dispatch through `Lv2Request` is a layering
                // bug -- the classifier never decodes this variant.
                self.record_invariant_break(
                    "dispatch.callback_dispatch_spawn_via_request",
                    format_args!(
                        "CallbackDispatchSpawn reached dispatch via Lv2Request; should be \
                         constructed only as Lv2Dispatch::CallbackSpawn from \
                         call_guest_callback_sync"
                    ),
                );
                Lv2Dispatch::Immediate {
                    code: errno::CELL_EINVAL.into(),
                    effects: vec![],
                }
            }
            Lv2Request::CallbackDispatchReturn { args } => {
                self.dispatch_callback_return(requester, args)
            }
            Lv2Request::Hypercall { lev, r11, args } => {
                // PS3 usermode never issues `sc` with LEV != 0;
                // reject with CELL_EINVAL rather than letting the
                // call fall through to LV2.
                self.log_invariant_break(
                    "dispatch.hypercall_rejected",
                    format_args!(
                        "sc LEV={lev} r11={r11:#x} from PS3 usermode; \
                         hypercalls are a programming error \
                         (r3={:#x} r4={:#x} r5={:#x} r6={:#x} r7={:#x} r8={:#x} r9={:#x} r10={:#x})",
                        args[0], args[1], args[2], args[3], args[4], args[5], args[6], args[7],
                    ),
                );
                Lv2Dispatch::Immediate {
                    code: errno::CELL_EINVAL.into(),
                    effects: vec![],
                }
            }
            Lv2Request::Unsupported { number, args } => {
                self.log_invariant_break(
                    "dispatch.unsupported_stub",
                    format_args!(
                        "syscall {number} has no dispatch handler (r3={:#x} r4={:#x} r5={:#x} \
                         r6={:#x} r7={:#x} r8={:#x} r9={:#x} r10={:#x}); returning CELL_OK stub \
                         (guests keying on errno for this syscall will misbehave)",
                        args[0], args[1], args[2], args[3], args[4], args[5], args[6], args[7],
                    ),
                );
                Lv2Dispatch::Immediate {
                    code: 0u64,
                    effects: vec![],
                }
            }
            Lv2Request::Malformed {
                number,
                reason,
                args,
            } => {
                self.log_invariant_break(
                    "dispatch.malformed_syscall",
                    format_args!(
                        "syscall {number} rejected: {reason} (r3={:#x} r4={:#x} r5={:#x} \
                         r6={:#x} r7={:#x} r8={:#x} r9={:#x} r10={:#x})",
                        args[0], args[1], args[2], args[3], args[4], args[5], args[6], args[7],
                    ),
                );
                Lv2Dispatch::Immediate {
                    code: errno::CELL_EINVAL.into(),
                    effects: vec![],
                }
            }
        }
    }

    /// Append the TTY buffer into [`Self::tty_log`] and write
    /// `nwritten` back. An unmapped buffer skips the append and
    /// still reports `len` written.
    pub(super) fn dispatch_tty_write(
        &mut self,
        buf_ptr: u32,
        len: u32,
        nwritten_ptr: u32,
        requester: UnitId,
        rt: &dyn Lv2Runtime,
    ) -> Lv2Dispatch {
        if len > 0 {
            if let Some(bytes) = rt.read_committed(buf_ptr as u64, len as usize) {
                self.tty_log.extend_from_slice(bytes);
            }
        }
        self.immediate_write_u32(len, nwritten_ptr, requester)
    }

    /// Per-class active-object count for
    /// `sys_process_get_number_of_object`. Maps `sys_process.h`
    /// `SYS_*_OBJECT` ids onto CellGov's tables; unmodeled classes
    /// report zero. Writes a 32-bit count (PSL1GHT `size_t` is 4
    /// bytes in PPU64 ILP32).
    pub(super) fn dispatch_process_get_number_of_object(
        &self,
        class_id: u32,
        count_out_ptr: u32,
        source: UnitId,
    ) -> Lv2Dispatch {
        let count: u32 = match class_id {
            0x85 => self.mutexes.len() as u32,      // SYS_MUTEX_OBJECT
            0x86 => self.conds.len() as u32, // SYS_COND_OBJECT (heavy cond, syscall 105 path)
            0x88 => self.rwlock_count,       // SYS_RWLOCK_OBJECT
            0x0E => self.event_port_count,   // SYS_EVENT_PORT_OBJECT
            0x11 => self.timer_count,        // SYS_TIMER_OBJECT
            0x8D => self.event_queues.len() as u32, // SYS_EVENT_QUEUE_OBJECT
            0x95 => self.lwmutexes.len() as u32, // SYS_LWMUTEX_OBJECT
            0x96 => self.semaphores.len() as u32, // SYS_SEMAPHORE_OBJECT
            0x97 => self.lwcond_count,       // SYS_LWCOND_OBJECT
            0x73 => self.fs_fd_count,        // SYS_FS_FD_OBJECT
            0x98 => self.event_flags.len() as u32, // SYS_EVENT_FLAG_OBJECT
            _ => 0,
        };
        self.immediate_write_u32(count, count_out_ptr, source)
    }

    /// Writes `0xFFFFFFFF` -- the value real PS3 reports for
    /// PSL1GHT-built homebrew with no SDK version recorded.
    pub(super) fn dispatch_process_get_sdk_version(
        &self,
        version_out_ptr: u32,
        source: UnitId,
    ) -> Lv2Dispatch {
        let version: u32 = 0xFFFF_FFFF;
        let write = Effect::SharedWriteIntent {
            range: ByteRange::new(GuestAddr::new(version_out_ptr as u64), 4).unwrap(),
            bytes: WritePayload::from_slice(&version.to_be_bytes()),
            ordering: PriorityClass::Normal,
            source,
            source_time: self.current_tick,
        };
        Lv2Dispatch::Immediate {
            code: 0,
            effects: vec![write],
        }
    }

    /// Writes the 64-byte SFO blob real PS3 returns for
    /// PSL1GHT-built homebrew with no PARAM.SFO loaded: version=1
    /// at byte 0, parental_level=4 at byte 23, attribute=1 at byte
    /// 31, rest zero.
    pub(super) fn dispatch_process_get_paramsfo(
        &self,
        buf_ptr: u32,
        source: UnitId,
    ) -> Lv2Dispatch {
        let mut blob = [0u8; 64];
        blob[0] = 0x01;
        blob[23] = 0x04;
        blob[31] = 0x01;
        let write = Effect::SharedWriteIntent {
            range: ByteRange::new(GuestAddr::new(buf_ptr as u64), 64).unwrap(),
            bytes: WritePayload::from_slice(&blob),
            ordering: PriorityClass::Normal,
            source,
            source_time: self.current_tick,
        };
        Lv2Dispatch::Immediate {
            code: 0,
            effects: vec![write],
        }
    }

    /// Build an immediate dispatch that writes `value` (BE u32) to
    /// `ptr` and returns CELL_OK; shared by create-style syscalls
    /// that emit a freshly allocated id through an out-pointer.
    pub(super) fn immediate_write_u32(&self, value: u32, ptr: u32, source: UnitId) -> Lv2Dispatch {
        let write = Effect::SharedWriteIntent {
            range: ByteRange::new(GuestAddr::new(ptr as u64), 4).unwrap(),
            bytes: WritePayload::from_slice(&value.to_be_bytes()),
            ordering: PriorityClass::Normal,
            source,
            source_time: self.current_tick,
        };
        Lv2Dispatch::Immediate {
            code: 0,
            effects: vec![write],
        }
    }
}

#[cfg(test)]
#[path = "tests/host_tests.rs"]
mod cross_primitive_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::test_support::{primary_attrs, FakeRuntime};

    #[test]
    fn tty_write_writes_nwritten_and_returns_ok() {
        let mut host = Lv2Host::new();
        let rt = FakeRuntime::new(0x10000);
        let req = Lv2Request::TtyWrite {
            fd: 0,
            buf_ptr: 0x8000,
            len: 64,
            nwritten_ptr: 0x9000,
        };
        let result = host.dispatch(req, UnitId::new(0), &rt);
        match result {
            Lv2Dispatch::Immediate { code, effects } => {
                assert_eq!(code, 0);
                assert_eq!(effects.len(), 1);
                if let Effect::SharedWriteIntent { range, bytes, .. } = &effects[0] {
                    assert_eq!(range.start().raw(), 0x9000);
                    assert_eq!(range.length(), 4);
                    assert_eq!(bytes.bytes(), &64u32.to_be_bytes());
                } else {
                    panic!("expected SharedWriteIntent");
                }
            }
            other => panic!("expected Immediate, got {other:?}"),
        }
    }

    #[test]
    fn tty_write_appends_buffer_bytes_to_tty_log() {
        let mut mem = cellgov_mem::GuestMemory::new(0x10000);
        mem.apply_commit(
            ByteRange::new(GuestAddr::new(0x8000), 12).unwrap(),
            b"hello world\n",
        )
        .unwrap();
        let rt = FakeRuntime::with_memory(mem);
        let mut host = Lv2Host::new();
        host.dispatch(
            Lv2Request::TtyWrite {
                fd: 1,
                buf_ptr: 0x8000,
                len: 12,
                nwritten_ptr: 0x9000,
            },
            UnitId::new(0),
            &rt,
        );
        assert_eq!(host.tty_log(), b"hello world\n");
    }

    #[test]
    fn tty_write_concatenates_across_calls_in_dispatch_order() {
        let mut mem = cellgov_mem::GuestMemory::new(0x10000);
        mem.apply_commit(ByteRange::new(GuestAddr::new(0x8000), 4).unwrap(), b"abcd")
            .unwrap();
        mem.apply_commit(ByteRange::new(GuestAddr::new(0x8100), 3).unwrap(), b"xyz")
            .unwrap();
        let rt = FakeRuntime::with_memory(mem);
        let mut host = Lv2Host::new();
        host.dispatch(
            Lv2Request::TtyWrite {
                fd: 1,
                buf_ptr: 0x8000,
                len: 4,
                nwritten_ptr: 0x9000,
            },
            UnitId::new(0),
            &rt,
        );
        host.dispatch(
            Lv2Request::TtyWrite {
                fd: 1,
                buf_ptr: 0x8100,
                len: 3,
                nwritten_ptr: 0x9000,
            },
            UnitId::new(0),
            &rt,
        );
        assert_eq!(host.tty_log(), b"abcdxyz");
    }

    #[test]
    fn tty_write_zero_len_is_a_noop_for_tty_log() {
        let rt = FakeRuntime::new(0x10000);
        let mut host = Lv2Host::new();
        host.dispatch(
            Lv2Request::TtyWrite {
                fd: 1,
                buf_ptr: 0x8000,
                len: 0,
                nwritten_ptr: 0x9000,
            },
            UnitId::new(0),
            &rt,
        );
        assert!(host.tty_log().is_empty());
    }

    #[test]
    fn tty_write_unmapped_buf_does_not_corrupt_tty_log_and_still_returns_ok() {
        let rt = FakeRuntime::new(0x1000);
        let mut host = Lv2Host::new();
        let result = host.dispatch(
            Lv2Request::TtyWrite {
                fd: 1,
                buf_ptr: 0x8000,
                len: 4,
                nwritten_ptr: 0x100,
            },
            UnitId::new(0),
            &rt,
        );
        match result {
            Lv2Dispatch::Immediate { code, .. } => assert_eq!(code, 0),
            other => panic!("expected Immediate, got {other:?}"),
        }
        assert!(host.tty_log().is_empty());
    }

    #[test]
    fn time_get_current_time_writes_sec_and_nsec_at_zero_tick() {
        let mut host = Lv2Host::new();
        let rt = FakeRuntime::new(0x10000);
        let result = host.dispatch(
            Lv2Request::TimeGetCurrentTime {
                sec_ptr: 0x8000,
                nsec_ptr: 0x8008,
            },
            UnitId::new(0),
            &rt,
        );
        match result {
            Lv2Dispatch::Immediate { code, effects } => {
                assert_eq!(code, 0);
                assert_eq!(effects.len(), 2);
                for eff in &effects {
                    if let Effect::SharedWriteIntent { range, bytes, .. } = eff {
                        assert_eq!(range.length(), 8);
                        assert_eq!(bytes.bytes(), &0u64.to_be_bytes());
                    } else {
                        panic!("expected SharedWriteIntent");
                    }
                }
            }
            other => panic!("expected Immediate, got {other:?}"),
        }
    }

    #[test]
    fn time_get_current_time_splits_at_billion_tick() {
        let mut host = Lv2Host::new();
        let rt = FakeRuntime::new(0x10000).with_tick(cellgov_time::GuestTicks::new(1_500_000_001));
        let result = host.dispatch(
            Lv2Request::TimeGetCurrentTime {
                sec_ptr: 0x1000,
                nsec_ptr: 0x1008,
            },
            UnitId::new(0),
            &rt,
        );
        let effects = match result {
            Lv2Dispatch::Immediate { effects, .. } => effects,
            other => panic!("expected Immediate, got {other:?}"),
        };
        if let Effect::SharedWriteIntent { bytes, .. } = &effects[0] {
            let v = u64::from_be_bytes(bytes.bytes().try_into().unwrap());
            assert_eq!(v, 1);
        } else {
            panic!();
        }
        if let Effect::SharedWriteIntent { bytes, .. } = &effects[1] {
            let v = u64::from_be_bytes(bytes.bytes().try_into().unwrap());
            assert_eq!(v, 500_000_001);
        } else {
            panic!();
        }
    }

    #[test]
    fn time_get_current_time_and_timebase_frequency_are_coherent() {
        let tick_later = 3 * cellgov_time::SIMULATED_INSTRUCTIONS_PER_SECOND + 500_000_000;
        let (sec, nsec) = cellgov_time::ticks_to_sec_nsec(tick_later);
        let tb = cellgov_time::ticks_to_tb(tick_later);
        let as_nsec_from_time_syscall = sec * 1_000_000_000 + nsec;
        let us_from_tb = tb * 1_000_000 / cellgov_time::CELL_PPU_TIMEBASE_HZ;
        let nsec_from_tb = us_from_tb * 1_000;
        // TB granularity is ~12.5 ns; require agreement under 1 us.
        let diff = as_nsec_from_time_syscall.abs_diff(nsec_from_tb);
        assert!(
            diff < 1_000,
            "time syscall and mftb must agree within 1 us: got {diff} ns"
        );
    }

    #[test]
    fn time_get_timebase_frequency_returns_cell_ppu_timebase_hz() {
        let mut host = Lv2Host::new();
        let rt = FakeRuntime::new(256);
        let result = host.dispatch(Lv2Request::TimeGetTimebaseFrequency, UnitId::new(0), &rt);
        assert_eq!(
            result,
            Lv2Dispatch::Immediate {
                code: cellgov_time::CELL_PPU_TIMEBASE_HZ,
                effects: vec![],
            }
        );
        assert_eq!(cellgov_time::CELL_PPU_TIMEBASE_HZ, 79_800_000);
    }

    #[test]
    fn cell_ps3_user_memory_total_is_213_mib() {
        // 213 MiB == 0x0D50_0000 == 223,346,688 bytes (PS3 game-mode
        // user-memory cap).
        assert_eq!(cellgov_ps3_abi::sys_memory::USER_MEMORY_TOTAL, 0x0D50_0000);
        assert_eq!(cellgov_ps3_abi::sys_memory::USER_MEMORY_TOTAL, 223_346_688);
    }

    #[test]
    fn time_get_timezone_writes_zero_through_both_out_pointers() {
        let mut host = Lv2Host::new();
        let rt = FakeRuntime::new(256);
        let result = host.dispatch(
            Lv2Request::TimeGetTimezone {
                timezone_ptr: 0xd000_fd10,
                summer_time_ptr: 0xd000_fd14,
            },
            UnitId::new(0),
            &rt,
        );
        match result {
            Lv2Dispatch::Immediate { code, effects } => {
                assert_eq!(code, 0);
                assert_eq!(effects.len(), 2);
                let expected_zero = 0i32.to_be_bytes();
                if let Effect::SharedWriteIntent { range, bytes, .. } = &effects[0] {
                    assert_eq!(range.start().raw(), 0xd000_fd10);
                    assert_eq!(range.length(), 4);
                    assert_eq!(bytes.bytes(), &expected_zero);
                } else {
                    panic!("expected SharedWriteIntent for timezone_ptr");
                }
                if let Effect::SharedWriteIntent { range, bytes, .. } = &effects[1] {
                    assert_eq!(range.start().raw(), 0xd000_fd14);
                    assert_eq!(range.length(), 4);
                    assert_eq!(bytes.bytes(), &expected_zero);
                } else {
                    panic!("expected SharedWriteIntent for summer_time_ptr");
                }
            }
            other => panic!("expected Immediate, got {other:?}"),
        }
    }

    #[test]
    fn stub_dispatch_returns_cell_ok_for_process_exit() {
        let mut host = Lv2Host::new();
        let rt = FakeRuntime::new(256);
        let req = Lv2Request::ProcessExit { code: 0 };
        let result = host.dispatch(req, UnitId::new(0), &rt);
        assert_eq!(
            result,
            Lv2Dispatch::Immediate {
                code: 0,
                effects: vec![],
            }
        );
    }

    #[test]
    fn stub_dispatch_returns_cell_ok_for_unsupported() {
        let mut host = Lv2Host::new();
        let rt = FakeRuntime::new(256);
        let req = Lv2Request::Unsupported {
            number: 999,
            args: [0; 8],
        };
        let result = host.dispatch(req, UnitId::new(0), &rt);
        assert_eq!(
            result,
            Lv2Dispatch::Immediate {
                code: 0,
                effects: vec![],
            }
        );
    }

    #[test]
    fn tty_read_returns_eio() {
        let mut host = Lv2Host::new();
        let rt = FakeRuntime::new(256);
        let result = host.dispatch(
            Lv2Request::Unsupported {
                number: 402,
                args: [0; 8],
            },
            UnitId::new(0),
            &rt,
        );
        assert_eq!(
            result,
            Lv2Dispatch::Immediate {
                code: errno::CELL_EIO.into(),
                effects: vec![],
            }
        );
    }

    #[test]
    fn prx_start_module_returns_einval() {
        let mut host = Lv2Host::new();
        let rt = FakeRuntime::new(256);
        let result = host.dispatch(
            Lv2Request::Unsupported {
                number: 481,
                args: [0; 8],
            },
            UnitId::new(0),
            &rt,
        );
        assert_eq!(
            result,
            Lv2Dispatch::Immediate {
                code: errno::CELL_EINVAL.into(),
                effects: vec![],
            }
        );
    }

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
    fn state_hash_unchanged_when_ppu_table_empty() {
        let fresh = Lv2Host::new();
        assert_eq!(fresh.state_hash(), Lv2Host::new().state_hash());
    }

    #[test]
    fn state_hash_changes_after_primary_seed() {
        let pre_seed = Lv2Host::new().state_hash();
        let mut seeded = Lv2Host::new();
        seeded.seed_primary_ppu_thread(UnitId::new(0), primary_attrs());
        assert_ne!(pre_seed, seeded.state_hash());
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
    fn state_hash_unchanged_when_tls_template_empty() {
        let fresh = Lv2Host::new();
        assert_eq!(fresh.state_hash(), Lv2Host::new().state_hash());
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
    fn state_hash_changes_when_holds_inserted_then_returns_to_baseline() {
        let mut host = Lv2Host::new();
        host.seed_primary_ppu_thread(UnitId::new(0), primary_attrs());
        let baseline = host.state_hash();
        let tid = host.ppu_thread_id_for_unit(UnitId::new(0)).unwrap();
        host.lwmutex_holds_inc(tid);
        assert_ne!(baseline, host.state_hash());
        host.lwmutex_holds_dec(tid);
        assert_eq!(baseline, host.state_hash());
    }

    #[test]
    fn state_hash_changes_after_tls_template_set() {
        let pre = Lv2Host::new().state_hash();
        let mut host = Lv2Host::new();
        host.set_tls_template(crate::ppu_thread::TlsTemplate::new(
            vec![0x11, 0x22],
            0x80,
            0x10,
            0x1000,
        ));
        assert_ne!(pre, host.state_hash());
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
    fn state_hash_unchanged_when_no_child_stack_allocated() {
        let fresh = Lv2Host::new();
        assert_eq!(fresh.state_hash(), Lv2Host::new().state_hash());
    }

    #[test]
    fn state_hash_changes_after_child_stack_allocated() {
        let pre = Lv2Host::new().state_hash();
        let mut host = Lv2Host::new();
        let _ = host.allocate_child_stack(0x10_000, 0x10).unwrap();
        assert_ne!(pre, host.state_hash());
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
}
