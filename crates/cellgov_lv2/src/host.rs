//! `Lv2Host` -- the LV2 model the runtime calls into.
//!
//! The host owns image registry and thread group state. The runtime
//! calls `dispatch` once per syscall yield, synchronously, during the
//! same `step()` that processed the yield. The host reads guest memory
//! through the `Lv2Runtime` trait and returns an `Lv2Dispatch` telling
//! the runtime what to do.

pub use self::rsx::{
    SysRsxContext, PACKAGE_CELLGOV_SET_FLIP_HANDLER, PACKAGE_CELLGOV_SET_USER_HANDLER,
    PACKAGE_CELLGOV_SET_VBLANK_HANDLER, PACKAGE_FLIP_BUFFER, PACKAGE_FLIP_MODE,
    PACKAGE_SET_DISPLAY_BUFFER,
};
use crate::dispatch::Lv2Dispatch;
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
use cellgov_time::GuestTicks;

/// Readonly view of runtime state exposed to the host.
///
/// The runtime implements this trait. The host calls `read_committed`
/// to read guest memory during dispatch and `current_tick` to stamp
/// LV2-sourced effects at the guest time the triggering syscall was
/// issued. No other runtime internals are exposed.
pub trait Lv2Runtime {
    /// Read `len` bytes of committed guest memory starting at `addr`.
    /// Returns `None` if the range is out of bounds.
    fn read_committed(&self, addr: u64, len: usize) -> Option<&[u8]>;

    /// The global guest tick at which the current dispatch is being
    /// handled. LV2-sourced effects use this as their `source_time`
    /// so they participate in commit-pipeline ordering at the tick
    /// of the triggering syscall rather than at tick 0.
    fn current_tick(&self) -> GuestTicks;
}

mod cond;
mod event_flag;
mod event_queue;
mod lwmutex;
mod memory;
mod mutex;
mod ppu_thread;
mod rsx;
mod semaphore;
mod spu;

#[cfg(test)]
mod test_support;

/// The LV2 host model. The runtime calls [`Self::dispatch`] on
/// every PPU syscall yield; the host mutates its own state and
/// returns an [`Lv2Dispatch`].
#[derive(Debug, Clone)]
pub struct Lv2Host {
    content: ContentStore,
    groups: ThreadGroupTable,
    ppu_threads: PpuThreadTable,
    tls_template: TlsTemplate,
    stack_allocator: ThreadStackAllocator,
    /// Monotonic kernel-object id counter, shared across
    /// `mutexes` / `semaphores` / `event_queues` / `event_flags` /
    /// `conds`. Starts at `0x4000_0001`. `lwmutexes` has its own
    /// allocator starting at 1.
    next_kernel_id: u32,
    mem_alloc_ptr: u32,
    /// sys_rsx_memory_allocate bump cursor. Grows upward from
    /// [`Self::SYS_RSX_MEM_BASE`]; separate from `mem_alloc_ptr` so
    /// the RSX-visible region cannot collide with PPU allocations.
    rsx_mem_alloc_ptr: u32,
    /// Monotonic handle counter returned from sys_rsx_memory_allocate.
    rsx_mem_handle_counter: u32,
    /// sys_rsx context bookkeeping. Populated by
    /// `sys_rsx_context_allocate` and consumed by subsequent
    /// `sys_rsx_context_attribute` dispatches. RPCS3 supports one
    /// context per renderer and CellGov follows suit; see
    /// `sys_rsx.cpp:251`.
    rsx_context: SysRsxContext,
    lwmutexes: LwMutexTable,
    mutexes: MutexTable,
    semaphores: SemaphoreTable,
    event_queues: EventQueueTable,
    event_flags: EventFlagTable,
    conds: CondTable,
    /// Dispatch-local scratch: set from
    /// [`Lv2Runtime::current_tick`] at the top of each `dispatch`
    /// call and stamped onto every LV2-sourced
    /// [`Effect::SharedWriteIntent`] as `source_time` so commits
    /// land at the triggering syscall's tick. Not folded into
    /// [`Self::state_hash`].
    current_tick: GuestTicks,
    /// Counter for host-invariant breaks caught defensively by
    /// [`Self::record_invariant_break`] /
    /// [`Self::log_invariant_break`]. Zero in a clean run;
    /// non-zero means at least one primitive wake or table update
    /// fell back to a degraded response in release. Not hashed.
    invariant_break_count: usize,
}

impl Default for Lv2Host {
    fn default() -> Self {
        Self::new()
    }
}

impl Lv2Host {
    /// Guest base address for sys_rsx memory allocations. Chosen so
    /// the 256 MB RSX-visible window sits above the 16 MB ELF /
    /// user-memory bumper and below the MEM_ALLOC_REGION_END guard
    /// used by `dispatch_memory_allocate`.
    pub const SYS_RSX_MEM_BASE: u32 = 0x3000_0000;

    /// Upper bound (exclusive) of the sys_rsx memory region. Matches
    /// real PS3's 256 MB RSX-visible size.
    pub const SYS_RSX_MEM_END: u32 = Self::SYS_RSX_MEM_BASE + 0x1000_0000;

    /// Construct an empty host.
    pub fn new() -> Self {
        Self {
            content: ContentStore::new(),
            groups: ThreadGroupTable::new(),
            ppu_threads: PpuThreadTable::new(),
            tls_template: TlsTemplate::empty(),
            stack_allocator: ThreadStackAllocator::new(),
            next_kernel_id: 0x4000_0001, // start above zero to catch uninitialized use
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
        }
    }

    /// Running total of invariant breaks caught by the crate-private
    /// `record_invariant_break` / `log_invariant_break` helpers.
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

    /// Log-once without `debug_assert!`. For paths reachable by
    /// guest input under normal operation (the `Unsupported`
    /// syscall fallthrough): at this stage of development,
    /// real boots hit unhandled syscalls routinely, so panicking
    /// on the first one would block development.
    fn log_invariant_break(&mut self, site: &'static str, details: std::fmt::Arguments<'_>) {
        if self.invariant_break_count == 0 {
            eprintln!("lv2 host invariant break at {site}: {details}");
        }
        self.invariant_break_count = self.invariant_break_count.saturating_add(1);
    }

    /// Resolve a `PpuThreadId` popped from a primitive waiter list
    /// to its `UnitId`. `None` means the thread table and the
    /// primitive diverged: logged via
    /// [`Self::record_invariant_break`] so the caller can fall
    /// back defensively.
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

    /// Override the allocator cursor. `run-game` must set this to
    /// the 64KB-aligned address above the ELF's highest PT_LOAD
    /// end; the default (`0x0001_0000`) assumes no ELF is loaded
    /// and will overwrite the image otherwise.
    pub fn set_mem_alloc_base(&mut self, base: u32) {
        self.mem_alloc_ptr = base;
    }

    /// Read the current sys_rsx context bookkeeping.
    #[inline]
    pub fn sys_rsx_context(&self) -> &SysRsxContext {
        &self.rsx_context
    }

    pub(super) fn alloc_id(&mut self) -> u32 {
        let id = self.next_kernel_id;
        self.next_kernel_id += 1;
        id
    }

    /// SPU image registry.
    pub fn content_store(&self) -> &ContentStore {
        &self.content
    }

    /// Mutable image registry; test harnesses pre-register images.
    pub fn content_store_mut(&mut self) -> &mut ContentStore {
        &mut self.content
    }

    /// SPU thread group table.
    pub fn thread_groups(&self) -> &ThreadGroupTable {
        &self.groups
    }

    /// Mutable thread group table (tests).
    pub fn thread_groups_mut(&mut self) -> &mut ThreadGroupTable {
        &mut self.groups
    }

    /// PPU thread table.
    pub fn ppu_threads(&self) -> &PpuThreadTable {
        &self.ppu_threads
    }

    /// Mutable PPU thread table (tests).
    pub fn ppu_threads_mut(&mut self) -> &mut PpuThreadTable {
        &mut self.ppu_threads
    }

    /// Seed the primary PPU thread. Call exactly once after the
    /// primary PPU unit is registered; `sys_ppu_thread_create`
    /// handles every subsequent thread.
    pub fn seed_primary_ppu_thread(&mut self, unit_id: UnitId, attrs: PpuThreadAttrs) {
        self.ppu_threads.insert_primary(unit_id, attrs);
    }

    /// Look up a PPU thread by runtime `UnitId`. `None` when the
    /// unit is not a PPU thread (e.g. SPU).
    pub fn ppu_thread_for_unit(&self, unit_id: UnitId) -> Option<&PpuThread> {
        self.ppu_threads.get_by_unit(unit_id)
    }

    /// Resolve a runtime `UnitId` to its guest-facing `PpuThreadId`.
    pub fn ppu_thread_id_for_unit(&self, unit_id: UnitId) -> Option<PpuThreadId> {
        self.ppu_threads.thread_id_for_unit(unit_id)
    }

    /// Capture the game ELF's PT_TLS template (empty by default).
    pub fn set_tls_template(&mut self, template: TlsTemplate) {
        self.tls_template = template;
    }

    /// Captured PT_TLS template.
    pub fn tls_template(&self) -> &TlsTemplate {
        &self.tls_template
    }

    /// Lightweight mutex table.
    pub fn lwmutexes(&self) -> &LwMutexTable {
        &self.lwmutexes
    }

    /// Mutable lwmutex table (tests).
    pub fn lwmutexes_mut(&mut self) -> &mut LwMutexTable {
        &mut self.lwmutexes
    }

    /// Heavy mutex table.
    pub fn mutexes(&self) -> &MutexTable {
        &self.mutexes
    }

    /// Mutable mutex table (tests).
    pub fn mutexes_mut(&mut self) -> &mut MutexTable {
        &mut self.mutexes
    }

    /// Counting semaphore table.
    pub fn semaphores(&self) -> &SemaphoreTable {
        &self.semaphores
    }

    /// Mutable semaphore table (tests).
    pub fn semaphores_mut(&mut self) -> &mut SemaphoreTable {
        &mut self.semaphores
    }

    /// Event queue table.
    pub fn event_queues(&self) -> &EventQueueTable {
        &self.event_queues
    }

    /// Mutable event queue table (tests).
    pub fn event_queues_mut(&mut self) -> &mut EventQueueTable {
        &mut self.event_queues
    }

    /// Event flag table.
    pub fn event_flags(&self) -> &EventFlagTable {
        &self.event_flags
    }

    /// Cond table.
    pub fn conds(&self) -> &CondTable {
        &self.conds
    }

    /// Mutable cond table (tests).
    pub fn conds_mut(&mut self) -> &mut CondTable {
        &mut self.conds
    }

    /// Mutable event flag table (tests).
    pub fn event_flags_mut(&mut self) -> &mut EventFlagTable {
        &mut self.event_flags
    }

    /// Reserve a child-thread stack block (16-byte aligned by
    /// default).
    pub fn allocate_child_stack(&mut self, size: u64, align: u64) -> Option<ThreadStack> {
        self.stack_allocator.allocate(size, align)
    }

    /// Record that `unit_id` is an SPU in `group_id` at `slot`.
    pub fn record_spu(
        &mut self,
        unit_id: cellgov_event::UnitId,
        group_id: u32,
        slot: u32,
    ) -> Result<(), crate::thread_group::RecordSpuError> {
        self.groups.record_spu(unit_id, group_id, slot)
    }

    /// Notify that the SPU `unit_id` has finished. Returns the
    /// group id if this completes the group (all SPUs done).
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
    /// Per-primitive tables (`ppu_threads`, `tls_template`,
    /// `lwmutexes`, `mutexes`, `semaphores`, `event_queues`,
    /// `event_flags`, `conds`) and the child-stack allocator
    /// cursor fold in only when non-empty / past their sentinel.
    /// `next_kernel_id` and `mem_alloc_ptr` always contribute, so
    /// a created-then-destroyed primitive still advances the hash
    /// via allocator state even when the table empties again.
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
        hasher.finish()
    }

    /// Dispatch a syscall request; called once per PPU syscall
    /// yield.
    pub fn dispatch(
        &mut self,
        request: Lv2Request,
        requester: UnitId,
        rt: &dyn Lv2Runtime,
    ) -> Lv2Dispatch {
        // current_tick is read by every helper below so LV2-
        // sourced effects stamp at the triggering syscall's tick.
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
                // Routed separately from Join (177) so the ABI
                // shape distinction is preserved at dispatch
                // time. No SPU teardown is modelled; observable
                // effect matches the existing join stub.
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
                len, nwritten_ptr, ..
            } => self.immediate_write_u32(len, nwritten_ptr, requester),
            Lv2Request::LwMutexCreate { id_ptr, .. } => {
                self.dispatch_lwmutex_create(id_ptr, requester)
            }
            Lv2Request::LwMutexDestroy { id } => self.dispatch_lwmutex_destroy(id),
            Lv2Request::LwMutexLock { id, .. } => self.dispatch_lwmutex_lock(id, requester),
            Lv2Request::LwMutexUnlock { id } => self.dispatch_lwmutex_unlock(id, requester),
            Lv2Request::LwMutexTryLock { id } => self.dispatch_lwmutex_trylock(id, requester),
            Lv2Request::MutexCreate { id_ptr, attr_ptr } => {
                self.dispatch_mutex_create(id_ptr, attr_ptr, requester, rt)
            }
            Lv2Request::MutexLock { mutex_id, .. } => self.dispatch_mutex_lock(mutex_id, requester),
            Lv2Request::MutexUnlock { mutex_id } => self.dispatch_mutex_unlock(mutex_id, requester),
            Lv2Request::MutexTryLock { mutex_id } => {
                self.dispatch_mutex_trylock(mutex_id, requester)
            }
            Lv2Request::SemaphoreCreate {
                id_ptr,
                initial,
                max,
                ..
            } => self.dispatch_semaphore_create(id_ptr, initial, max, requester),
            Lv2Request::SemaphoreDestroy { id } => self.dispatch_semaphore_destroy(id),
            Lv2Request::SemaphoreWait { id, .. } => self.dispatch_semaphore_wait(id, requester),
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
            Lv2Request::EventFlagCreate { id_ptr, init, .. } => {
                self.dispatch_event_flag_create(id_ptr, init, requester)
            }
            Lv2Request::EventFlagDestroy { id } => self.dispatch_event_flag_destroy(id),
            Lv2Request::EventFlagWait {
                id,
                bits,
                mode,
                result_ptr,
                ..
            } => self.dispatch_event_flag_wait(id, bits, mode, result_ptr, requester),
            Lv2Request::EventFlagTryWait {
                id,
                bits,
                mode,
                result_ptr,
            } => self.dispatch_event_flag_trywait(id, bits, mode, result_ptr, requester),
            Lv2Request::EventFlagSet { id, bits } => self.dispatch_event_flag_set(id, bits),
            Lv2Request::EventFlagClear { id, bits } => self.dispatch_event_flag_clear(id, bits),
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
                // Stub: no deallocation tracking. Double-free,
                // invalid pointer, and NULL all return CELL_OK.
                // Titles that key on free's errno will misbehave.
                Lv2Dispatch::Immediate {
                    code: 0u64,
                    effects: vec![],
                }
            }
            Lv2Request::MemoryGetUserMemorySize { mem_info_ptr } => {
                let total: u32 = 0x0D50_0000;
                let avail: u32 = 0x0D50_0000;
                let mut buf = [0u8; 8];
                buf[0..4].copy_from_slice(&total.to_be_bytes());
                buf[4..8].copy_from_slice(&avail.to_be_bytes());
                let write = Effect::SharedWriteIntent {
                    range: ByteRange::new(GuestAddr::new(mem_info_ptr as u64), 8).unwrap(),
                    bytes: WritePayload::from_slice(&buf),
                    ordering: PriorityClass::Normal,
                    source: requester,
                    source_time: self.current_tick,
                };
                Lv2Dispatch::Immediate {
                    code: 0,
                    effects: vec![write],
                }
            }
            Lv2Request::MemoryContainerCreate { cid_ptr, .. } => {
                let id = self.alloc_id();
                self.immediate_write_u32(id, cid_ptr, requester)
            }
            Lv2Request::PpuThreadYield => Lv2Dispatch::Immediate {
                // Pure scheduler hint -- the round-robin walk
                // advances on the syscall yield itself.
                code: 0,
                effects: vec![],
            },
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
            // _sys_prx_start_module: RPCS3 returns EINVAL when
            // id == 0 or pOpt is null. CellGov's _sys_prx_load_module
            // stub returns id=0, so liblv2 calls start with id=0;
            // a CELL_OK stub here makes liblv2 read uninitialized
            // stack memory and crash on a bogus pointer.
            Lv2Request::Unsupported { number: 481, .. } => Lv2Dispatch::Immediate {
                code: crate::errno::CELL_EINVAL.into(),
                effects: vec![],
            },
            // sys_tty_read: RPCS3 returns EIO when debug console
            // mode is off (matches retail). A CELL_OK stub spins
            // CRT input loops indefinitely.
            Lv2Request::Unsupported { number: 402, .. } => Lv2Dispatch::Immediate {
                code: crate::errno::CELL_EIO.into(),
                effects: vec![],
            },
            // Runtime's prior `matches!` drives the Finished
            // cascade inside `handle_immediate`; the dispatch
            // itself is a plain Immediate.
            Lv2Request::ProcessExit { .. } => Lv2Dispatch::Immediate {
                code: 0u64,
                effects: vec![],
            },
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
                    code: crate::errno::CELL_EINVAL.into(),
                    effects: vec![],
                }
            }
        }
    }

    /// Immediate dispatch that writes a u32 to `ptr` and returns
    /// CELL_OK. Used by create-style syscalls that allocate a
    /// kernel object id and return it through an out-pointer.
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
        // Syscall 402 is sys_tty_read. RPCS3 returns CELL_EIO =
        // 0x8001002B outside debug console mode; that is the retail
        // behavior real games target. CELL_OK with no data causes
        // CRT input loops to spin indefinitely.
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
                code: crate::errno::CELL_EIO.into(),
                effects: vec![],
            }
        );
    }

    #[test]
    fn prx_start_module_returns_einval() {
        // Syscall 481 is _sys_prx_start_module. With id=0 or a null
        // pOpt, RPCS3 (and real LV2) returns CELL_EINVAL = 0x80010002.
        // Our stub always returns CELL_EINVAL because we do not track
        // PRX lifecycle state; this keeps liblv2 on a spec-correct
        // error path rather than CELL_OK-with-garbage-output.
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
                code: crate::errno::CELL_EINVAL.into(),
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
        // Regression guard: an empty PpuThreadTable must not
        // perturb Lv2Host::state_hash. Without this, every host
        // without a seeded primary thread would see its hash
        // change just because the PPU thread table field exists
        // on the struct.
        let fresh = Lv2Host::new();
        // Two fresh hosts produce identical hashes (sanity).
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
        // Regression guard matching the ppu_threads gating: an
        // empty TlsTemplate must not perturb state_hash. Without
        // this, hosts constructed before the loader captures a
        // template would see their hash shift just because the
        // field exists on the struct.
        let fresh = Lv2Host::new();
        assert_eq!(fresh.state_hash(), Lv2Host::new().state_hash());
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
        // Start at the 0xD0010000 child-stack base.
        assert_eq!(s1.base, 0xD001_0000);
        // Monotonic and non-overlapping.
        assert!(s2.base >= s1.end());
        assert!(s3.base >= s2.end());
    }

    #[test]
    fn state_hash_unchanged_when_no_child_stack_allocated() {
        // Regression guard: a fresh host (no child threads
        // spawned) must report the same hash it would before
        // the stack allocator field existed. Once
        // `allocate_child_stack` has advanced the cursor past
        // the sentinel, the contribution kicks in.
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
}
