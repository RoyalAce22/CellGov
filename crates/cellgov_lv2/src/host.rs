//! `Lv2Host` -- the LV2 model the runtime calls into.
//!
//! The host owns image registry and thread group state. The runtime
//! calls `dispatch` once per syscall yield, synchronously, during the
//! same `step()` that processed the yield. The host reads guest memory
//! through the `Lv2Runtime` trait and returns an `Lv2Dispatch` telling
//! the runtime what to do.

use crate::dispatch::{CondMutexKind, Lv2Dispatch, PendingResponse};
use crate::image::ContentStore;
use crate::ppu_thread::{
    PpuThread, PpuThreadAttrs, PpuThreadId, PpuThreadTable, ThreadStack, ThreadStackAllocator,
    TlsTemplate,
};
use crate::request::Lv2Request;
use crate::sync_primitives::{
    CondTable, EventFlagTable, EventPayload, EventQueueTable, LwMutexTable, MutexAttrs, MutexTable,
    SemaphoreTable,
};
use crate::thread_group::ThreadGroupTable;
use cellgov_effects::{Effect, WritePayload};
use cellgov_event::{PriorityClass, UnitId};
use cellgov_mem::{ByteRange, GuestAddr};
use cellgov_time::GuestTicks;

/// Readonly view of runtime state exposed to the host.
///
/// The runtime implements this trait. The host calls `read_committed`
/// to read guest memory during dispatch. No other runtime internals
/// are exposed.
pub trait Lv2Runtime {
    /// Read `len` bytes of committed guest memory starting at `addr`.
    /// Returns `None` if the range is out of bounds.
    fn read_committed(&self, addr: u64, len: usize) -> Option<&[u8]>;
}

mod spu;

/// The LV2 host model.
///
/// Holds all LV2-side state: image registry, thread group table,
/// waiter lists. The runtime owns an `Lv2Host` and calls `dispatch`
/// on every PPU syscall yield.
#[derive(Debug, Clone)]
pub struct Lv2Host {
    content: ContentStore,
    groups: ThreadGroupTable,
    /// PPU threads (primary plus any created via
    /// `sys_ppu_thread_create`). Populated by
    /// `seed_primary_ppu_thread` after the primary PPU unit is
    /// registered with the runtime. Empty until seeded.
    ppu_threads: PpuThreadTable,
    /// Captured PT_TLS template, populated by `set_tls_template`
    /// when the loader parses the game ELF. Empty on a freshly
    /// constructed host and for games with no PT_TLS segment.
    /// Used by `sys_ppu_thread_create` to stamp a fresh TLS
    /// block for each child thread.
    tls_template: TlsTemplate,
    /// Allocator for child-thread stack blocks above
    /// `0xD0010000`. Starts fresh per host; deterministic.
    stack_allocator: ThreadStackAllocator,
    /// Monotonic ID counter for kernel objects (mutexes, event queues, etc.),
    /// starting at base `0x40000001`.
    next_kernel_id: u32,
    /// Bump allocator for sys_memory_allocate. Hands out 64KB-aligned
    /// addresses from the PS3 user-memory region starting at
    /// 0x00010000 (per PSDevWiki and RPCS3 `vm.cpp`).
    mem_alloc_ptr: u32,
    /// Lightweight-mutex table. Empty until the first
    /// `sys_lwmutex_create`. Its id space is independent of
    /// `next_kernel_id`; lwmutex ids start at 1.
    lwmutexes: LwMutexTable,
    /// Heavy-mutex table. Empty until the first
    /// `sys_mutex_create`. Ids are minted by the shared
    /// `next_kernel_id` allocator; the table stores by that id.
    mutexes: MutexTable,
    /// Counting semaphore table. Empty until the first
    /// `sys_semaphore_create`. Ids are minted by
    /// `next_kernel_id` (distinct from `mutexes` because the
    /// BTreeMap key spaces are local to each table).
    semaphores: SemaphoreTable,
    /// Event queue table. Empty until the first
    /// `sys_event_queue_create`. Ids are minted by
    /// `next_kernel_id`; event-port ids are treated as queue ids
    /// directly (one-to-one binding, matching the common
    /// pattern).
    event_queues: EventQueueTable,
    /// Event flag table. Empty until the first
    /// `sys_event_flag_create`. Ids are minted by
    /// `next_kernel_id`.
    event_flags: EventFlagTable,
    /// Condition-variable table. Empty until the first
    /// `sys_cond_create`. Ids are minted by `next_kernel_id`.
    conds: CondTable,
}

impl Default for Lv2Host {
    fn default() -> Self {
        Self::new()
    }
}

impl Lv2Host {
    /// Construct an empty host with no registered images, groups,
    /// or PPU threads.
    pub fn new() -> Self {
        Self {
            content: ContentStore::new(),
            groups: ThreadGroupTable::new(),
            ppu_threads: PpuThreadTable::new(),
            tls_template: TlsTemplate::empty(),
            stack_allocator: ThreadStackAllocator::new(),
            next_kernel_id: 0x4000_0001, // start above zero to catch uninitialized use
            mem_alloc_ptr: 0x0001_0000,  // PS3 user-memory region start
            lwmutexes: LwMutexTable::new(),
            mutexes: MutexTable::new(),
            semaphores: SemaphoreTable::new(),
            event_queues: EventQueueTable::new(),
            event_flags: EventFlagTable::new(),
            conds: CondTable::new(),
        }
    }

    /// Override the allocator's next-returnable address.
    ///
    /// Real PS3 LV2 shares the `0x00010000-0x0FFFFFFF` user-memory
    /// region between the loaded ELF PT_LOAD segments and the
    /// `sys_memory_allocate` pool. The kernel tracks what the ELF
    /// occupies and hands out addresses past it. The default base
    /// (`0x00010000`) assumes no ELF is loaded; `run-game` and any
    /// caller that loads a real game must call this with the
    /// 64KB-aligned address immediately above the ELF's highest
    /// PT_LOAD end, so allocations do not overwrite the image.
    pub fn set_mem_alloc_base(&mut self, base: u32) {
        self.mem_alloc_ptr = base;
    }

    /// Allocate a monotonic kernel object ID.
    fn alloc_id(&mut self) -> u32 {
        let id = self.next_kernel_id;
        self.next_kernel_id += 1;
        id
    }

    /// Borrow the content store (image registry).
    pub fn content_store(&self) -> &ContentStore {
        &self.content
    }

    /// Mutably borrow the content store. The test harness calls this
    /// to pre-register SPU images before the scenario runs.
    pub fn content_store_mut(&mut self) -> &mut ContentStore {
        &mut self.content
    }

    /// Borrow the thread group table.
    pub fn thread_groups(&self) -> &ThreadGroupTable {
        &self.groups
    }

    /// Mutably borrow the thread group table.
    pub fn thread_groups_mut(&mut self) -> &mut ThreadGroupTable {
        &mut self.groups
    }

    /// Borrow the PPU thread table.
    pub fn ppu_threads(&self) -> &PpuThreadTable {
        &self.ppu_threads
    }

    /// Mutably borrow the PPU thread table.
    pub fn ppu_threads_mut(&mut self) -> &mut PpuThreadTable {
        &mut self.ppu_threads
    }

    /// Seed the primary PPU thread.
    ///
    /// Called by the runtime after registering the primary PPU
    /// execution unit. Associates `unit_id` with
    /// `PpuThreadId::PRIMARY` (0x0100_0000) and records the
    /// attributes captured from the ELF entry. Must be called
    /// exactly once; panics if the primary thread is already
    /// seeded. Subsequent PPU threads are created via
    /// `sys_ppu_thread_create`.
    pub fn seed_primary_ppu_thread(&mut self, unit_id: UnitId, attrs: PpuThreadAttrs) {
        self.ppu_threads.insert_primary(unit_id, attrs);
    }

    /// Look up a PPU thread by its runtime unit id. Returns `None`
    /// if the unit is not a PPU thread (e.g., it's an SPU).
    pub fn ppu_thread_for_unit(&self, unit_id: UnitId) -> Option<&PpuThread> {
        self.ppu_threads.get_by_unit(unit_id)
    }

    /// Look up the guest-facing PPU thread id for a runtime unit
    /// id. Returns `None` if the unit is not a PPU thread.
    pub fn ppu_thread_id_for_unit(&self, unit_id: UnitId) -> Option<PpuThreadId> {
        self.ppu_threads.thread_id_for_unit(unit_id)
    }

    /// Capture the game ELF's PT_TLS template. Called by the CLI's
    /// boot path once per process, after `find_tls_segment`
    /// returns a PT_TLS header. Safe to call with an empty
    /// template for games without PT_TLS (the default).
    pub fn set_tls_template(&mut self, template: TlsTemplate) {
        self.tls_template = template;
    }

    /// Borrow the captured TLS template. The
    /// `sys_ppu_thread_create` handler uses this to stamp a
    /// fresh per-thread TLS block for each child.
    pub fn tls_template(&self) -> &TlsTemplate {
        &self.tls_template
    }

    /// Read-only view of the lightweight mutex table.
    pub fn lwmutexes(&self) -> &LwMutexTable {
        &self.lwmutexes
    }

    /// Mutable access to the lightweight mutex table. Tests use
    /// this to preload state; dispatch handlers mutate through the
    /// host's own private paths.
    pub fn lwmutexes_mut(&mut self) -> &mut LwMutexTable {
        &mut self.lwmutexes
    }

    /// Read-only view of the heavy mutex table.
    pub fn mutexes(&self) -> &MutexTable {
        &self.mutexes
    }

    /// Mutable access to the heavy mutex table. Tests use this to
    /// preload state.
    pub fn mutexes_mut(&mut self) -> &mut MutexTable {
        &mut self.mutexes
    }

    /// Read-only view of the counting semaphore table.
    pub fn semaphores(&self) -> &SemaphoreTable {
        &self.semaphores
    }

    /// Mutable access to the counting semaphore table. Tests use
    /// this to preload state.
    pub fn semaphores_mut(&mut self) -> &mut SemaphoreTable {
        &mut self.semaphores
    }

    /// Read-only view of the event queue table.
    pub fn event_queues(&self) -> &EventQueueTable {
        &self.event_queues
    }

    /// Mutable access to the event queue table. Tests use this to
    /// preload state.
    pub fn event_queues_mut(&mut self) -> &mut EventQueueTable {
        &mut self.event_queues
    }

    /// Read-only view of the event flag table.
    pub fn event_flags(&self) -> &EventFlagTable {
        &self.event_flags
    }

    /// Borrow the cond table.
    pub fn conds(&self) -> &CondTable {
        &self.conds
    }

    /// Mutably borrow the cond table. Used by tests that preload
    /// cond entries; the runtime mutates the table via `dispatch`.
    pub fn conds_mut(&mut self) -> &mut CondTable {
        &mut self.conds
    }

    /// Mutable access to the event flag table. Tests use this to
    /// preload state.
    pub fn event_flags_mut(&mut self) -> &mut EventFlagTable {
        &mut self.event_flags
    }

    /// Allocate a child-thread stack block. Called by
    /// `sys_ppu_thread_create` to reserve a deterministic
    /// address range for each new PPU thread. The returned
    /// block is 16-byte-aligned by default; pass a larger
    /// alignment for specific layout requirements.
    pub fn allocate_child_stack(&mut self, size: u64, align: u64) -> Option<ThreadStack> {
        self.stack_allocator.allocate(size, align)
    }

    /// Record that `unit_id` is an SPU belonging to `group_id` at
    /// `slot`. The runtime calls this after each SPU is registered
    /// during `RegisterSpu` handling.
    pub fn record_spu(&mut self, unit_id: cellgov_event::UnitId, group_id: u32, slot: u32) {
        self.groups.record_spu(unit_id, group_id, slot);
    }

    /// Notify that the SPU `unit_id` has finished. Returns
    /// `Some(group_id)` if the group is now fully finished (all
    /// SPUs done), `None` otherwise.
    pub fn notify_spu_finished(&mut self, unit_id: cellgov_event::UnitId) -> Option<u32> {
        self.groups.notify_spu_finished(unit_id)
    }

    /// FNV-1a hash of all LV2 host state for determinism checking.
    ///
    /// The runtime folds this into `sync_state_hash` at every commit
    /// boundary so replay tooling detects LV2 state divergence.
    pub fn state_hash(&self) -> u64 {
        let mut hasher = cellgov_mem::Fnv1aHasher::new();
        for source in [self.content.state_hash(), self.groups.state_hash()] {
            hasher.write(&source.to_le_bytes());
        }
        hasher.write(&self.next_kernel_id.to_le_bytes());
        hasher.write(&self.mem_alloc_ptr.to_le_bytes());
        // PPU thread table is folded in only when non-empty so
        // that host instances with no seeded primary thread keep
        // the same hash they had before the table field existed.
        // Callers that seed the primary and spawn children see
        // every state change flow into the hash.
        if !self.ppu_threads.is_empty() {
            hasher.write(&self.ppu_threads.state_hash().to_le_bytes());
        }
        // Same gating for the TLS template: a freshly constructed
        // host with no captured template keeps the pre-existing
        // hash. Once the loader calls `set_tls_template`, the
        // template contents contribute.
        if !self.tls_template.is_empty() {
            hasher.write(&self.tls_template.state_hash().to_le_bytes());
        }
        // Stack allocator cursor is folded in only after the
        // first child-stack allocation (cursor moves past the
        // CHILD_STACK_BASE sentinel). A host that never spawns
        // a child keeps the pre-existing hash; once a child is
        // spawned the cursor contributes.
        if let Some(peek) = self.stack_allocator.peek_next(0x10) {
            if peek != ThreadStackAllocator::CHILD_STACK_BASE {
                hasher.write(&peek.to_le_bytes());
            }
        }
        // Lwmutex table is folded in only when non-empty so that
        // hosts that never create a lwmutex (every current
        // foundation title during boot) keep the hash they had
        // before the table field existed.
        if !self.lwmutexes.is_empty() {
            hasher.write(&self.lwmutexes.state_hash().to_le_bytes());
        }
        // Heavy mutex table is folded in only when non-empty for
        // the same reason. Note: historical foundation-title runs
        // did call sys_mutex_create (the old stub allocated ids
        // from next_kernel_id without storing anything in a
        // table); those hashes are preserved below because the
        // upgraded handler still uses next_kernel_id for id
        // minting and is_empty() only fires once a real entry
        // is stored.
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

    /// Dispatch a syscall request.
    ///
    /// The runtime calls this once per PPU syscall yield. The host
    /// reads any guest memory it needs through `rt`, mutates its own
    /// internal state, and returns an `Lv2Dispatch` describing what
    /// the runtime should do.
    pub fn dispatch(
        &mut self,
        request: Lv2Request,
        requester: UnitId,
        rt: &dyn Lv2Runtime,
    ) -> Lv2Dispatch {
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
            } => self.dispatch_group_join(group_id, cause_ptr, status_ptr),
            Lv2Request::SpuThreadWriteMb { thread_id, value } => {
                self.dispatch_write_mb(thread_id, value, requester)
            }
            Lv2Request::TtyWrite {
                len, nwritten_ptr, ..
            } => Self::immediate_write_u32(len, nwritten_ptr, requester),
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
                // Stub: no-op (CellGov doesn't track memory deallocation).
                Lv2Dispatch::Immediate {
                    code: 0,
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
                    source_time: GuestTicks::ZERO,
                };
                Lv2Dispatch::Immediate {
                    code: 0,
                    effects: vec![write],
                }
            }
            Lv2Request::MemoryContainerCreate { cid_ptr, .. } => {
                let id = self.alloc_id();
                Self::immediate_write_u32(id, cid_ptr, requester)
            }
            Lv2Request::PpuThreadYield => {
                // Pure hint: return CELL_OK with no effects. The
                // scheduler's round-robin walk already moves on
                // to a different runnable unit after a syscall
                // yield; the handler does not need to touch unit
                // state.
                Lv2Dispatch::Immediate {
                    code: 0,
                    effects: vec![],
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
            } => self.dispatch_ppu_thread_create(id_ptr, entry_opd, arg, priority, stacksize),
            Lv2Request::PpuThreadJoin {
                target,
                status_out_ptr,
            } => self.dispatch_ppu_thread_join(target, status_out_ptr, requester),
            // Syscall 481 is _sys_prx_start_module. RPCS3 returns
            // CELL_EINVAL (0x80010002) when id == 0 or pOpt is null.
            // CellGov's _sys_prx_load_module stub returns 0 (id=0),
            // so liblv2 ends up calling start with id=0. Returning
            // CELL_EINVAL signals a real, spec-correct failure
            // instead of CELL_OK with an unfilled output struct --
            // the latter causes liblv2 to read uninitialized stack
            // memory and crash on a bogus pointer.
            Lv2Request::Unsupported { number: 481 } => Lv2Dispatch::Immediate {
                code: 0x8001_0002,
                effects: vec![],
            },
            // Syscall 402 is sys_tty_read. RPCS3 returns CELL_EIO
            // (0x8001002B) when debug console mode is not enabled,
            // which matches real retail PS3 behavior. A CELL_OK stub
            // tells the game the read succeeded with uninitialized
            // bytes, causing CRT input loops to spin indefinitely.
            Lv2Request::Unsupported { number: 402 } => Lv2Dispatch::Immediate {
                code: 0x8001_002B,
                effects: vec![],
            },
            _ => Lv2Dispatch::Immediate {
                code: 0,
                effects: vec![],
            },
        }
    }

    /// Build an Immediate dispatch that writes a u32 value to a guest
    /// pointer and returns CELL_OK. Used by create-style syscalls that
    /// allocate a kernel object and write its ID to an output parameter.
    fn immediate_write_u32(value: u32, ptr: u32, source: UnitId) -> Lv2Dispatch {
        let write = Effect::SharedWriteIntent {
            range: ByteRange::new(GuestAddr::new(ptr as u64), 4).unwrap(),
            bytes: WritePayload::from_slice(&value.to_be_bytes()),
            ordering: PriorityClass::Normal,
            source,
            source_time: GuestTicks::ZERO,
        };
        Lv2Dispatch::Immediate {
            code: 0,
            effects: vec![write],
        }
    }

    fn dispatch_mutex_create(
        &mut self,
        id_ptr: u32,
        attr_ptr: u32,
        requester: UnitId,
        rt: &dyn Lv2Runtime,
    ) -> Lv2Dispatch {
        // Decode the attribute bag if the guest handed us one. PS3
        // `sys_mutex_attribute_t` is a 32-byte struct where the
        // first three big-endian u32s encode protocol, recursive
        // flag, and pshared. The remaining fields (adaptive, name)
        // are captured but not surfaced at this layer.
        let attrs = if attr_ptr == 0 {
            MutexAttrs::default()
        } else if let Some(bytes) = rt.read_committed(attr_ptr as u64, 12) {
            let protocol = u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
            let recursive_raw = u32::from_be_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
            MutexAttrs {
                priority_policy: protocol,
                recursive: recursive_raw != 0,
                protocol,
            }
        } else {
            MutexAttrs::default()
        };
        let id = self.alloc_id();
        if !self.mutexes.create_with_id(id, attrs) {
            // Id collision should be impossible (next_kernel_id is
            // monotonic and unique), but if it ever happens surface
            // ENOMEM rather than silently drop.
            return Lv2Dispatch::Immediate {
                code: 0x8001_000C,
                effects: vec![],
            };
        }
        Self::immediate_write_u32(id, id_ptr, requester)
    }

    fn dispatch_mutex_lock(&mut self, id: u32, requester: UnitId) -> Lv2Dispatch {
        let Some(caller) = self.ppu_threads.thread_id_for_unit(requester) else {
            return Lv2Dispatch::Immediate {
                code: 0x8001_0005, // CELL_ESRCH
                effects: vec![],
            };
        };
        match self.mutexes.try_acquire(id, caller) {
            None => Lv2Dispatch::Immediate {
                code: 0x8001_0005, // CELL_ESRCH
                effects: vec![],
            },
            Some(crate::sync_primitives::MutexAcquire::Acquired) => Lv2Dispatch::Immediate {
                code: 0,
                effects: vec![],
            },
            Some(crate::sync_primitives::MutexAcquire::Contended) => {
                if !self.mutexes.enqueue_waiter(id, caller) {
                    return Lv2Dispatch::Immediate {
                        code: 0x8001_000D, // CELL_EDEADLK
                        effects: vec![],
                    };
                }
                Lv2Dispatch::Block {
                    reason: crate::dispatch::Lv2BlockReason::Mutex { id },
                    pending: PendingResponse::ReturnCode { code: 0 },
                    effects: vec![],
                }
            }
        }
    }

    fn dispatch_semaphore_create(
        &mut self,
        id_ptr: u32,
        initial: i32,
        max: i32,
        requester: UnitId,
    ) -> Lv2Dispatch {
        // EINVAL (0x8001_0002) for initial > max or negative values.
        // The table also validates this but the syscall-level
        // check surfaces the guest error cleanly before any id
        // allocation.
        if initial > max || initial < 0 || max < 0 {
            return Lv2Dispatch::Immediate {
                code: 0x8001_0002,
                effects: vec![],
            };
        }
        let id = self.alloc_id();
        if !self.semaphores.create_with_id(id, initial, max) {
            // Allocator collision (impossible with monotonic
            // next_kernel_id) or validation tripped -- return
            // ENOMEM.
            return Lv2Dispatch::Immediate {
                code: 0x8001_000C,
                effects: vec![],
            };
        }
        Self::immediate_write_u32(id, id_ptr, requester)
    }

    fn dispatch_semaphore_destroy(&mut self, id: u32) -> Lv2Dispatch {
        let Some(entry) = self.semaphores.lookup(id) else {
            return Lv2Dispatch::Immediate {
                code: 0x8001_0005, // CELL_ESRCH
                effects: vec![],
            };
        };
        if !entry.waiters().is_empty() {
            return Lv2Dispatch::Immediate {
                code: 0x8001_000A, // CELL_EBUSY
                effects: vec![],
            };
        }
        self.semaphores.destroy(id);
        Lv2Dispatch::Immediate {
            code: 0,
            effects: vec![],
        }
    }

    fn dispatch_semaphore_wait(&mut self, id: u32, requester: UnitId) -> Lv2Dispatch {
        let Some(caller) = self.ppu_threads.thread_id_for_unit(requester) else {
            return Lv2Dispatch::Immediate {
                code: 0x8001_0005, // CELL_ESRCH
                effects: vec![],
            };
        };
        match self.semaphores.try_wait(id) {
            None => Lv2Dispatch::Immediate {
                code: 0x8001_0005, // CELL_ESRCH
                effects: vec![],
            },
            Some(crate::sync_primitives::SemaphoreWait::Acquired) => Lv2Dispatch::Immediate {
                code: 0,
                effects: vec![],
            },
            Some(crate::sync_primitives::SemaphoreWait::Empty) => {
                if !self.semaphores.enqueue_waiter(id, caller) {
                    return Lv2Dispatch::Immediate {
                        code: 0x8001_000D, // CELL_EDEADLK (duplicate enqueue)
                        effects: vec![],
                    };
                }
                Lv2Dispatch::Block {
                    reason: crate::dispatch::Lv2BlockReason::Semaphore { id },
                    pending: PendingResponse::ReturnCode { code: 0 },
                    effects: vec![],
                }
            }
        }
    }

    fn dispatch_semaphore_trywait(&mut self, id: u32) -> Lv2Dispatch {
        match self.semaphores.try_wait(id) {
            None => Lv2Dispatch::Immediate {
                code: 0x8001_0005, // CELL_ESRCH
                effects: vec![],
            },
            Some(crate::sync_primitives::SemaphoreWait::Acquired) => Lv2Dispatch::Immediate {
                code: 0,
                effects: vec![],
            },
            Some(crate::sync_primitives::SemaphoreWait::Empty) => Lv2Dispatch::Immediate {
                code: 0x8001_000A, // CELL_EBUSY
                effects: vec![],
            },
        }
    }

    fn dispatch_semaphore_get_value(
        &mut self,
        id: u32,
        out_ptr: u32,
        requester: UnitId,
    ) -> Lv2Dispatch {
        let Some(entry) = self.semaphores.lookup(id) else {
            return Lv2Dispatch::Immediate {
                code: 0x8001_0005, // CELL_ESRCH
                effects: vec![],
            };
        };
        let count = entry.count() as u32;
        Self::immediate_write_u32(count, out_ptr, requester)
    }

    fn dispatch_semaphore_post(&mut self, id: u32, val: i32) -> Lv2Dispatch {
        // Only val == 1 is supported. Multi-slot post would wake
        // multiple waiters in one dispatch, which complicates the
        // WakeAndReturn protocol; deferred.
        if val != 1 {
            return Lv2Dispatch::Immediate {
                code: 0x8001_0002, // CELL_EINVAL
                effects: vec![],
            };
        }
        match self.semaphores.post_and_wake(id) {
            crate::sync_primitives::SemaphorePost::Unknown => Lv2Dispatch::Immediate {
                code: 0x8001_0005, // CELL_ESRCH
                effects: vec![],
            },
            crate::sync_primitives::SemaphorePost::OverMax => Lv2Dispatch::Immediate {
                code: 0x8001_0002, // CELL_EINVAL
                effects: vec![],
            },
            crate::sync_primitives::SemaphorePost::Incremented => Lv2Dispatch::Immediate {
                code: 0,
                effects: vec![],
            },
            crate::sync_primitives::SemaphorePost::Woke { new_owner } => {
                if let Some(thread) = self.ppu_threads.get(new_owner) {
                    Lv2Dispatch::WakeAndReturn {
                        code: 0,
                        woken_unit_ids: vec![thread.unit_id],
                        response_updates: vec![],
                        effects: vec![],
                    }
                } else {
                    // Woken thread no longer in the ppu_threads
                    // table -- defensive fallback. Post still
                    // succeeded (the waiter came off the list)
                    // but no unit to wake.
                    Lv2Dispatch::Immediate {
                        code: 0,
                        effects: vec![],
                    }
                }
            }
        }
    }

    fn dispatch_mutex_trylock(&mut self, id: u32, requester: UnitId) -> Lv2Dispatch {
        let Some(caller) = self.ppu_threads.thread_id_for_unit(requester) else {
            return Lv2Dispatch::Immediate {
                code: 0x8001_0005, // CELL_ESRCH
                effects: vec![],
            };
        };
        match self.mutexes.try_acquire(id, caller) {
            None => Lv2Dispatch::Immediate {
                code: 0x8001_0005, // CELL_ESRCH
                effects: vec![],
            },
            Some(crate::sync_primitives::MutexAcquire::Acquired) => Lv2Dispatch::Immediate {
                code: 0,
                effects: vec![],
            },
            Some(crate::sync_primitives::MutexAcquire::Contended) => Lv2Dispatch::Immediate {
                code: 0x8001_000A, // CELL_EBUSY
                effects: vec![],
            },
        }
    }

    fn dispatch_mutex_unlock(&mut self, id: u32, requester: UnitId) -> Lv2Dispatch {
        let Some(caller) = self.ppu_threads.thread_id_for_unit(requester) else {
            return Lv2Dispatch::Immediate {
                code: 0x8001_0005, // CELL_ESRCH
                effects: vec![],
            };
        };
        match self.mutexes.release_and_wake_next(id, caller) {
            crate::sync_primitives::MutexRelease::Unknown => Lv2Dispatch::Immediate {
                code: 0x8001_0005, // CELL_ESRCH
                effects: vec![],
            },
            crate::sync_primitives::MutexRelease::NotOwner => Lv2Dispatch::Immediate {
                code: 0x8001_0008, // CELL_EPERM
                effects: vec![],
            },
            crate::sync_primitives::MutexRelease::Freed => Lv2Dispatch::Immediate {
                code: 0,
                effects: vec![],
            },
            crate::sync_primitives::MutexRelease::Transferred { new_owner } => {
                if let Some(thread) = self.ppu_threads.get(new_owner) {
                    Lv2Dispatch::WakeAndReturn {
                        code: 0,
                        woken_unit_ids: vec![thread.unit_id],
                        response_updates: vec![],
                        effects: vec![],
                    }
                } else {
                    Lv2Dispatch::Immediate {
                        code: 0,
                        effects: vec![],
                    }
                }
            }
        }
    }

    fn dispatch_lwmutex_create(&mut self, id_ptr: u32, requester: UnitId) -> Lv2Dispatch {
        // Allocate from the lwmutex table's private id allocator
        // (starts at 1, independent of `next_kernel_id`). On
        // overflow return ENOMEM (CELL_ENOMEM = 0x8001_000C) and
        // do not write the out pointer.
        let Some(id) = self.lwmutexes.create() else {
            return Lv2Dispatch::Immediate {
                code: 0x8001_000C,
                effects: vec![],
            };
        };
        Self::immediate_write_u32(id, id_ptr, requester)
    }

    fn dispatch_lwmutex_lock(&mut self, id: u32, requester: UnitId) -> Lv2Dispatch {
        // The caller's PPU thread id is required for ownership
        // tracking. If the requesting unit is not registered as a
        // PPU thread the syscall is a guest programming error --
        // reject with ESRCH rather than silently park.
        let Some(caller) = self.ppu_threads.thread_id_for_unit(requester) else {
            return Lv2Dispatch::Immediate {
                code: 0x8001_0005, // CELL_ESRCH
                effects: vec![],
            };
        };
        match self.lwmutexes.try_acquire(id, caller) {
            None => Lv2Dispatch::Immediate {
                code: 0x8001_0005, // CELL_ESRCH
                effects: vec![],
            },
            Some(crate::sync_primitives::LwMutexAcquire::Acquired) => Lv2Dispatch::Immediate {
                code: 0,
                effects: vec![],
            },
            Some(crate::sync_primitives::LwMutexAcquire::Contended) => {
                // Enqueue the caller on the waiter list. A duplicate
                // enqueue (same caller already parked) is a guest
                // error; the table rejects it and we return EDEADLK.
                if !self.lwmutexes.enqueue_waiter(id, caller) {
                    return Lv2Dispatch::Immediate {
                        code: 0x8001_000D, // CELL_EDEADLK
                        effects: vec![],
                    };
                }
                Lv2Dispatch::Block {
                    reason: crate::dispatch::Lv2BlockReason::LwMutex { id },
                    pending: PendingResponse::ReturnCode { code: 0 },
                    effects: vec![],
                }
            }
        }
    }

    fn dispatch_lwmutex_trylock(&mut self, id: u32, requester: UnitId) -> Lv2Dispatch {
        let Some(caller) = self.ppu_threads.thread_id_for_unit(requester) else {
            return Lv2Dispatch::Immediate {
                code: 0x8001_0005, // CELL_ESRCH
                effects: vec![],
            };
        };
        match self.lwmutexes.try_acquire(id, caller) {
            None => Lv2Dispatch::Immediate {
                code: 0x8001_0005, // CELL_ESRCH
                effects: vec![],
            },
            Some(crate::sync_primitives::LwMutexAcquire::Acquired) => Lv2Dispatch::Immediate {
                code: 0,
                effects: vec![],
            },
            Some(crate::sync_primitives::LwMutexAcquire::Contended) => Lv2Dispatch::Immediate {
                code: 0x8001_000A, // CELL_EBUSY
                effects: vec![],
            },
        }
    }

    fn dispatch_lwmutex_unlock(&mut self, id: u32, requester: UnitId) -> Lv2Dispatch {
        let Some(caller) = self.ppu_threads.thread_id_for_unit(requester) else {
            return Lv2Dispatch::Immediate {
                code: 0x8001_0005, // CELL_ESRCH
                effects: vec![],
            };
        };
        match self.lwmutexes.release_and_wake_next(id, caller) {
            crate::sync_primitives::LwMutexRelease::Unknown => Lv2Dispatch::Immediate {
                code: 0x8001_0005, // CELL_ESRCH
                effects: vec![],
            },
            crate::sync_primitives::LwMutexRelease::NotOwner => Lv2Dispatch::Immediate {
                code: 0x8001_0008, // CELL_EPERM
                effects: vec![],
            },
            crate::sync_primitives::LwMutexRelease::Freed => Lv2Dispatch::Immediate {
                code: 0,
                effects: vec![],
            },
            crate::sync_primitives::LwMutexRelease::Transferred { new_owner } => {
                // Resolve new_owner's PpuThreadId to a UnitId so the
                // runtime's WakeAndReturn handler can transition it
                // from Blocked back to Runnable. If the table no
                // longer knows the thread (destroyed mid-flight),
                // that is a determinism violation -- fall back to
                // Freed rather than orphan the owner.
                if let Some(thread) = self.ppu_threads.get(new_owner) {
                    Lv2Dispatch::WakeAndReturn {
                        code: 0,
                        woken_unit_ids: vec![thread.unit_id],
                        response_updates: vec![],
                        effects: vec![],
                    }
                } else {
                    Lv2Dispatch::Immediate {
                        code: 0,
                        effects: vec![],
                    }
                }
            }
        }
    }

    fn dispatch_lwmutex_destroy(&mut self, id: u32) -> Lv2Dispatch {
        // ESRCH for unknown id; EBUSY if any waiter is parked
        // (destroy-with-waiters is a guest programming error and
        // leaks waiters, so reject).
        let Some(entry) = self.lwmutexes.lookup(id) else {
            return Lv2Dispatch::Immediate {
                code: 0x8001_0005,
                effects: vec![],
            };
        };
        if !entry.waiters().is_empty() || entry.owner().is_some() {
            return Lv2Dispatch::Immediate {
                code: 0x8001_000A, // CELL_EBUSY
                effects: vec![],
            };
        }
        self.lwmutexes.destroy(id);
        Lv2Dispatch::Immediate {
            code: 0,
            effects: vec![],
        }
    }

    fn dispatch_event_queue_create(
        &mut self,
        id_ptr: u32,
        size: u32,
        requester: UnitId,
    ) -> Lv2Dispatch {
        // queue_size == 0 is guest-invalid; default to 127 (the
        // RPCS3 EQUEUE_MAX_RECV_EVENT) when the guest passes
        // zero, matching permissive ABI behavior.
        let effective_size = if size == 0 { 127 } else { size };
        let id = self.alloc_id();
        if !self.event_queues.create_with_id(id, effective_size) {
            return Lv2Dispatch::Immediate {
                code: 0x8001_000C, // CELL_ENOMEM
                effects: vec![],
            };
        }
        Self::immediate_write_u32(id, id_ptr, requester)
    }

    fn dispatch_event_queue_destroy(&mut self, id: u32) -> Lv2Dispatch {
        let Some(entry) = self.event_queues.lookup(id) else {
            return Lv2Dispatch::Immediate {
                code: 0x8001_0005, // CELL_ESRCH
                effects: vec![],
            };
        };
        if !entry.waiters().is_empty() {
            return Lv2Dispatch::Immediate {
                code: 0x8001_000A, // CELL_EBUSY
                effects: vec![],
            };
        }
        self.event_queues.destroy(id);
        Lv2Dispatch::Immediate {
            code: 0,
            effects: vec![],
        }
    }

    fn dispatch_event_queue_receive(
        &mut self,
        id: u32,
        out_ptr: u32,
        requester: UnitId,
    ) -> Lv2Dispatch {
        let Some(caller) = self.ppu_threads.thread_id_for_unit(requester) else {
            return Lv2Dispatch::Immediate {
                code: 0x8001_0005,
                effects: vec![],
            };
        };
        match self.event_queues.try_receive(id) {
            None => Lv2Dispatch::Immediate {
                code: 0x8001_0005, // CELL_ESRCH
                effects: vec![],
            },
            Some(crate::sync_primitives::EventQueueReceive::Delivered(payload)) => {
                // Payload available -- write the 4-u64
                // sys_event_t to out_ptr and return CELL_OK.
                let mut buf = [0u8; 32];
                buf[0..8].copy_from_slice(&payload.source.to_be_bytes());
                buf[8..16].copy_from_slice(&payload.data1.to_be_bytes());
                buf[16..24].copy_from_slice(&payload.data2.to_be_bytes());
                buf[24..32].copy_from_slice(&payload.data3.to_be_bytes());
                let write = Effect::SharedWriteIntent {
                    range: ByteRange::new(GuestAddr::new(out_ptr as u64), 32).unwrap(),
                    bytes: WritePayload::from_slice(&buf),
                    ordering: PriorityClass::Normal,
                    source: requester,
                    source_time: GuestTicks::ZERO,
                };
                Lv2Dispatch::Immediate {
                    code: 0,
                    effects: vec![write],
                }
            }
            Some(crate::sync_primitives::EventQueueReceive::Empty) => {
                if !self.event_queues.enqueue_waiter(id, caller, out_ptr) {
                    return Lv2Dispatch::Immediate {
                        code: 0x8001_000D, // CELL_EDEADLK
                        effects: vec![],
                    };
                }
                // Pending response carries the out_ptr the wait
                // handler received; payload fields are unused
                // placeholders. The send-side dispatch replaces
                // the full response via response_updates at wake
                // time and the runtime reads the complete value.
                Lv2Dispatch::Block {
                    reason: crate::dispatch::Lv2BlockReason::EventQueue { id },
                    pending: PendingResponse::EventQueueReceive {
                        out_ptr,
                        source: 0,
                        data1: 0,
                        data2: 0,
                        data3: 0,
                    },
                    effects: vec![],
                }
            }
        }
    }

    // Map the raw PS3 ABI `mode` word (sys_event_flag_wait_mode)
    // to the structured EventFlagWaitMode enum. The ABI encodes
    // policy as two independent bits:
    //   bit 0: 0 = AND, 1 = OR  (SYS_EVENT_FLAG_WAIT_OR = 0x02)
    //   bit 1: 0 = NO-CLEAR, 1 = CLEAR (SYS_EVENT_FLAG_WAIT_CLEAR = 0x10)
    // Real PS3 constants: AND=0x01, OR=0x02, CLEAR=0x10,
    // CLEAR_ALL=0x20. We accept both bit layouts: the common
    // (AND|CLEAR) = 0x11 value maps to AndClear, etc.
    fn decode_event_flag_mode(raw: u32) -> crate::ppu_thread::EventFlagWaitMode {
        let or_match = (raw & 0x02) != 0;
        let clear = (raw & 0x10) != 0;
        match (or_match, clear) {
            (false, false) => crate::ppu_thread::EventFlagWaitMode::AndNoClear,
            (false, true) => crate::ppu_thread::EventFlagWaitMode::AndClear,
            (true, false) => crate::ppu_thread::EventFlagWaitMode::OrNoClear,
            (true, true) => crate::ppu_thread::EventFlagWaitMode::OrClear,
        }
    }

    fn dispatch_event_flag_create(
        &mut self,
        id_ptr: u32,
        init: u64,
        requester: UnitId,
    ) -> Lv2Dispatch {
        let id = self.alloc_id();
        if !self.event_flags.create_with_id(id, init) {
            return Lv2Dispatch::Immediate {
                code: 0x8001_000C, // CELL_ENOMEM
                effects: vec![],
            };
        }
        Self::immediate_write_u32(id, id_ptr, requester)
    }

    fn dispatch_event_flag_destroy(&mut self, id: u32) -> Lv2Dispatch {
        let Some(entry) = self.event_flags.lookup(id) else {
            return Lv2Dispatch::Immediate {
                code: 0x8001_0005,
                effects: vec![],
            };
        };
        if !entry.waiters().is_empty() {
            return Lv2Dispatch::Immediate {
                code: 0x8001_000A,
                effects: vec![],
            };
        }
        self.event_flags.destroy(id);
        Lv2Dispatch::Immediate {
            code: 0,
            effects: vec![],
        }
    }

    fn dispatch_event_flag_wait(
        &mut self,
        id: u32,
        bits: u64,
        mode_raw: u32,
        result_ptr: u32,
        requester: UnitId,
    ) -> Lv2Dispatch {
        let Some(caller) = self.ppu_threads.thread_id_for_unit(requester) else {
            return Lv2Dispatch::Immediate {
                code: 0x8001_0005,
                effects: vec![],
            };
        };
        let mode = Self::decode_event_flag_mode(mode_raw);
        match self.event_flags.try_wait(id, bits, mode) {
            None => Lv2Dispatch::Immediate {
                code: 0x8001_0005,
                effects: vec![],
            },
            Some(crate::sync_primitives::EventFlagWait::Matched { observed }) => {
                // Bits matched -- write observed pattern to
                // result_ptr and return CELL_OK.
                let write = Effect::SharedWriteIntent {
                    range: ByteRange::new(GuestAddr::new(result_ptr as u64), 8).unwrap(),
                    bytes: WritePayload::from_slice(&observed.to_be_bytes()),
                    ordering: PriorityClass::Normal,
                    source: requester,
                    source_time: GuestTicks::ZERO,
                };
                Lv2Dispatch::Immediate {
                    code: 0,
                    effects: vec![write],
                }
            }
            Some(crate::sync_primitives::EventFlagWait::NoMatch) => {
                if !self
                    .event_flags
                    .enqueue_waiter(id, caller, bits, mode, result_ptr)
                {
                    return Lv2Dispatch::Immediate {
                        code: 0x8001_000D,
                        effects: vec![],
                    };
                }
                // Pending response placeholder; set-side dispatch
                // replaces it with a complete response carrying
                // the observed bits. The result_ptr stored on the
                // waiter entry is what makes that possible
                // without the runtime reading back the parked
                // response.
                Lv2Dispatch::Block {
                    reason: crate::dispatch::Lv2BlockReason::EventFlag { id },
                    pending: PendingResponse::EventFlagWake {
                        result_ptr,
                        observed: 0,
                    },
                    effects: vec![],
                }
            }
        }
    }

    fn dispatch_event_flag_trywait(
        &mut self,
        id: u32,
        bits: u64,
        mode_raw: u32,
        result_ptr: u32,
        requester: UnitId,
    ) -> Lv2Dispatch {
        let mode = Self::decode_event_flag_mode(mode_raw);
        match self.event_flags.try_wait(id, bits, mode) {
            None => Lv2Dispatch::Immediate {
                code: 0x8001_0005,
                effects: vec![],
            },
            Some(crate::sync_primitives::EventFlagWait::Matched { observed }) => {
                let write = Effect::SharedWriteIntent {
                    range: ByteRange::new(GuestAddr::new(result_ptr as u64), 8).unwrap(),
                    bytes: WritePayload::from_slice(&observed.to_be_bytes()),
                    ordering: PriorityClass::Normal,
                    source: requester,
                    source_time: GuestTicks::ZERO,
                };
                Lv2Dispatch::Immediate {
                    code: 0,
                    effects: vec![write],
                }
            }
            Some(crate::sync_primitives::EventFlagWait::NoMatch) => Lv2Dispatch::Immediate {
                code: 0x8001_000A, // CELL_EBUSY
                effects: vec![],
            },
        }
    }

    fn dispatch_event_flag_set(&mut self, id: u32, bits: u64) -> Lv2Dispatch {
        let Some(woken) = self.event_flags.set_and_wake(id, bits) else {
            return Lv2Dispatch::Immediate {
                code: 0x8001_0005,
                effects: vec![],
            };
        };
        if woken.is_empty() {
            return Lv2Dispatch::Immediate {
                code: 0,
                effects: vec![],
            };
        }
        // Build WakeAndReturn with one response_update per
        // woken waiter. `set_and_wake` returns the waiter's
        // recorded result_ptr alongside the observed bits, so
        // each response_update carries a complete
        // PendingResponse::EventFlagWake -- no merge magic.
        let mut unit_ids: Vec<UnitId> = Vec::new();
        let mut updates: Vec<(UnitId, PendingResponse)> = Vec::new();
        for wake in woken {
            if let Some(t) = self.ppu_threads.get(wake.thread) {
                unit_ids.push(t.unit_id);
                updates.push((
                    t.unit_id,
                    PendingResponse::EventFlagWake {
                        result_ptr: wake.result_ptr,
                        observed: wake.observed,
                    },
                ));
            }
        }
        Lv2Dispatch::WakeAndReturn {
            code: 0,
            woken_unit_ids: unit_ids,
            response_updates: updates,
            effects: vec![],
        }
    }

    fn dispatch_event_flag_clear(&mut self, id: u32, bits: u64) -> Lv2Dispatch {
        if !self.event_flags.clear_bits(id, bits) {
            return Lv2Dispatch::Immediate {
                code: 0x8001_0005,
                effects: vec![],
            };
        }
        Lv2Dispatch::Immediate {
            code: 0,
            effects: vec![],
        }
    }

    fn dispatch_cond_create(
        &mut self,
        id_ptr: u32,
        mutex_id: u32,
        requester: UnitId,
    ) -> Lv2Dispatch {
        // The cond binds to an existing heavy mutex. Reject at
        // create time if the mutex does not exist; this matches
        // RPCS3's behavior (lv2_obj::idm_get on the mutex id fails
        // with ESRCH).
        if self.mutexes.lookup(mutex_id).is_none() {
            return Lv2Dispatch::Immediate {
                code: 0x8001_0005, // CELL_ESRCH
                effects: vec![],
            };
        }
        let id = self.alloc_id();
        if !self
            .conds
            .create_with_id(id, mutex_id, CondMutexKind::Mutex)
        {
            return Lv2Dispatch::Immediate {
                code: 0x8001_000C, // CELL_ENOMEM
                effects: vec![],
            };
        }
        Self::immediate_write_u32(id, id_ptr, requester)
    }

    fn dispatch_cond_destroy(&mut self, id: u32) -> Lv2Dispatch {
        let Some(entry) = self.conds.lookup(id) else {
            return Lv2Dispatch::Immediate {
                code: 0x8001_0005, // CELL_ESRCH
                effects: vec![],
            };
        };
        if !entry.waiters().is_empty() {
            return Lv2Dispatch::Immediate {
                code: 0x8001_000A, // CELL_EBUSY
                effects: vec![],
            };
        }
        self.conds.destroy(id);
        Lv2Dispatch::Immediate {
            code: 0,
            effects: vec![],
        }
    }

    fn dispatch_cond_wait(&mut self, id: u32, requester: UnitId) -> Lv2Dispatch {
        let Some(caller) = self.ppu_threads.thread_id_for_unit(requester) else {
            return Lv2Dispatch::Immediate {
                code: 0x8001_0005, // CELL_ESRCH
                effects: vec![],
            };
        };
        let Some(entry) = self.conds.lookup(id) else {
            return Lv2Dispatch::Immediate {
                code: 0x8001_0005, // CELL_ESRCH
                effects: vec![],
            };
        };
        let mutex_id = entry.mutex_id();
        let mutex_kind = entry.mutex_kind();
        // Release the associated mutex on the caller's behalf. The
        // release is observable to any mutex waiter that was parked
        // -- ownership transfers, and that waiter wakes alongside
        // this cond-wait block.
        let release = match mutex_kind {
            CondMutexKind::Mutex => self.mutexes.release_and_wake_next(mutex_id, caller),
            CondMutexKind::LwMutex => {
                // sys_cond binds only to heavy mutexes here. A
                // lwmutex kind is a defensive fallback for
                // forward compatibility with sys_lwcond; treat as
                // EPERM so misuse is loud.
                return Lv2Dispatch::Immediate {
                    code: 0x8001_0008, // CELL_EPERM
                    effects: vec![],
                };
            }
        };
        match release {
            crate::sync_primitives::MutexRelease::Unknown => Lv2Dispatch::Immediate {
                code: 0x8001_0005, // CELL_ESRCH
                effects: vec![],
            },
            crate::sync_primitives::MutexRelease::NotOwner => Lv2Dispatch::Immediate {
                code: 0x8001_0008, // CELL_EPERM
                effects: vec![],
            },
            crate::sync_primitives::MutexRelease::Freed => {
                // Mutex had no waiter; simply park the caller on
                // the cond.
                if !self.conds.enqueue_waiter(id, caller) {
                    return Lv2Dispatch::Immediate {
                        code: 0x8001_000D, // CELL_EDEADLK (duplicate enqueue)
                        effects: vec![],
                    };
                }
                Lv2Dispatch::Block {
                    reason: crate::dispatch::Lv2BlockReason::Cond { id, mutex_id },
                    pending: PendingResponse::CondWakeReacquire {
                        mutex_id,
                        mutex_kind,
                    },
                    effects: vec![],
                }
            }
            crate::sync_primitives::MutexRelease::Transferred { new_owner } => {
                // Mutex waiter inherited ownership. The new owner
                // needs to wake in this same dispatch alongside the
                // caller blocking on the cond.
                if !self.conds.enqueue_waiter(id, caller) {
                    return Lv2Dispatch::Immediate {
                        code: 0x8001_000D,
                        effects: vec![],
                    };
                }
                let woken_unit_ids = if let Some(thread) = self.ppu_threads.get(new_owner) {
                    vec![thread.unit_id]
                } else {
                    vec![]
                };
                Lv2Dispatch::BlockAndWake {
                    reason: crate::dispatch::Lv2BlockReason::Cond { id, mutex_id },
                    pending: PendingResponse::CondWakeReacquire {
                        mutex_id,
                        mutex_kind,
                    },
                    woken_unit_ids,
                    response_updates: vec![],
                    effects: vec![],
                }
            }
        }
    }

    fn dispatch_cond_signal_all(&mut self, id: u32) -> Lv2Dispatch {
        let Some(entry) = self.conds.lookup(id) else {
            return Lv2Dispatch::Immediate {
                code: 0x8001_0005, // CELL_ESRCH
                effects: vec![],
            };
        };
        let mutex_id = entry.mutex_id();
        let mutex_kind = entry.mutex_kind();
        if !matches!(mutex_kind, CondMutexKind::Mutex) {
            return Lv2Dispatch::Immediate {
                code: 0x8001_0008, // CELL_EPERM
                effects: vec![],
            };
        }
        let wakers = self.conds.signal_all(id);
        if wakers.is_empty() {
            return Lv2Dispatch::Immediate {
                code: 0,
                effects: vec![],
            };
        }
        // Walk wakers in FIFO order. First acquirer (if mutex is
        // free at signal time) wakes cleanly; the rest re-park on
        // the mutex waiter list. Each gets its pending response
        // swapped to ReturnCode { 0 } so the unlock-wake path
        // resolves it as a plain CELL_OK return.
        let mut woken_unit_ids: Vec<UnitId> = Vec::new();
        let mut response_updates: Vec<(UnitId, PendingResponse)> = Vec::new();
        for waker in wakers {
            let Some(thread) = self.ppu_threads.get(waker) else {
                continue;
            };
            let unit = thread.unit_id;
            match self.mutexes.try_acquire(mutex_id, waker) {
                Some(crate::sync_primitives::MutexAcquire::Acquired) => {
                    woken_unit_ids.push(unit);
                    response_updates.push((unit, PendingResponse::ReturnCode { code: 0 }));
                }
                Some(crate::sync_primitives::MutexAcquire::Contended) => {
                    // Enqueue is a no-op if `waker` is already on
                    // the mutex waiter list -- a legitimate but
                    // unusual state (e.g. test harness seeds it
                    // directly, or the table was mutated out-of-
                    // band). The pending response still swaps so
                    // the eventual unlock-wake resolves the
                    // existing queue entry cleanly.
                    let _ = self.mutexes.enqueue_waiter(mutex_id, waker);
                    response_updates.push((unit, PendingResponse::ReturnCode { code: 0 }));
                }
                None => {
                    // Cond references a destroyed mutex; surface
                    // ESRCH on the waker's eventual wake rather
                    // than orphan them.
                    woken_unit_ids.push(unit);
                    response_updates
                        .push((unit, PendingResponse::ReturnCode { code: 0x8001_0005 }));
                }
            }
        }
        Lv2Dispatch::WakeAndReturn {
            code: 0,
            woken_unit_ids,
            response_updates,
            effects: vec![],
        }
    }

    fn dispatch_cond_signal_to(&mut self, id: u32, target_thread: u32) -> Lv2Dispatch {
        let Some(entry) = self.conds.lookup(id) else {
            return Lv2Dispatch::Immediate {
                code: 0x8001_0005, // CELL_ESRCH
                effects: vec![],
            };
        };
        let mutex_id = entry.mutex_id();
        let mutex_kind = entry.mutex_kind();
        if !matches!(mutex_kind, CondMutexKind::Mutex) {
            return Lv2Dispatch::Immediate {
                code: 0x8001_0008, // CELL_EPERM
                effects: vec![],
            };
        }
        let target = PpuThreadId::new(target_thread as u64);
        // Remove the target from the cond waiter list. If the
        // target is not parked on this cond, return ESRCH (lost
        // signal is not observable here -- the ABI distinguishes
        // "target not found" from signal_all's "no waiters" case
        // because the caller named a specific thread).
        if !self.conds.signal_to(id, target) {
            return Lv2Dispatch::Immediate {
                code: 0x8001_0005, // CELL_ESRCH
                effects: vec![],
            };
        }
        self.cond_reacquire_wake(target, mutex_id, false)
    }

    fn dispatch_cond_signal(&mut self, id: u32) -> Lv2Dispatch {
        let Some(entry) = self.conds.lookup(id) else {
            return Lv2Dispatch::Immediate {
                code: 0x8001_0005, // CELL_ESRCH
                effects: vec![],
            };
        };
        let mutex_id = entry.mutex_id();
        let mutex_kind = entry.mutex_kind();
        // Non-sticky: a signal with no parked waiter is lost.
        let Some(waker) = self.conds.signal_one(id) else {
            return Lv2Dispatch::Immediate {
                code: 0,
                effects: vec![],
            };
        };
        match mutex_kind {
            CondMutexKind::Mutex => self.cond_reacquire_wake(waker, mutex_id, false),
            CondMutexKind::LwMutex => Lv2Dispatch::Immediate {
                code: 0x8001_0008, // CELL_EPERM (sys_cond is heavy-only)
                effects: vec![],
            },
        }
    }

    /// Apply the cond-wake re-acquire protocol for a single thread.
    ///
    /// The woken thread holds a `PendingResponse::CondWakeReacquire`
    /// from the cond-wait side. This helper consults the mutex
    /// table and either:
    ///
    ///   * Acquires on the waker's behalf and wakes it via
    ///     `WakeAndRequire` with a swapped pending response
    ///     (`ReturnCode { 0 }`). Caller transitions Blocked ->
    ///     Runnable, r3 = 0.
    ///   * Re-parks the waker on the mutex's waiter list with a
    ///     swapped pending response, keeping it Blocked. The
    ///     signaler returns CELL_OK without waking anyone; the
    ///     waker wakes eventually when some other thread unlocks
    ///     the mutex.
    ///
    /// `use_lwmutex` selects the mutex table for lwcond support
    /// (not exercised by sys_cond).
    fn cond_reacquire_wake(
        &mut self,
        waker: PpuThreadId,
        mutex_id: u32,
        use_lwmutex: bool,
    ) -> Lv2Dispatch {
        debug_assert!(!use_lwmutex, "lwmutex cond re-acquire not wired");
        let Some(thread) = self.ppu_threads.get(waker) else {
            // Waker no longer tracked -- cond table and ppu thread
            // table diverged. Defensive fallback: signaler still
            // returns OK; the waker is effectively stranded.
            return Lv2Dispatch::Immediate {
                code: 0,
                effects: vec![],
            };
        };
        let waker_unit = thread.unit_id;
        match self.mutexes.try_acquire(mutex_id, waker) {
            Some(crate::sync_primitives::MutexAcquire::Acquired) => {
                // Mutex transferred to waker. Wake cleanly with
                // swapped pending response so the wake resolver
                // treats this as a plain CELL_OK return.
                Lv2Dispatch::WakeAndReturn {
                    code: 0,
                    woken_unit_ids: vec![waker_unit],
                    response_updates: vec![(waker_unit, PendingResponse::ReturnCode { code: 0 })],
                    effects: vec![],
                }
            }
            Some(crate::sync_primitives::MutexAcquire::Contended) => {
                // Mutex held by someone else. Re-park the waker on
                // the mutex waiter list with swapped pending
                // response. Waker stays Blocked; signaler returns
                // CELL_OK. When the mutex holder eventually calls
                // sys_mutex_unlock, the unlock-wake path transfers
                // ownership to this waker and resolves the pending
                // ReturnCode { 0 }. Enqueue is a no-op if the
                // waker is already present; the response swap
                // still lets the existing queue entry resolve
                // cleanly on unlock.
                let _ = self.mutexes.enqueue_waiter(mutex_id, waker);
                Lv2Dispatch::WakeAndReturn {
                    code: 0,
                    woken_unit_ids: vec![],
                    response_updates: vec![(waker_unit, PendingResponse::ReturnCode { code: 0 })],
                    effects: vec![],
                }
            }
            None => {
                // Mutex unknown -- cond still references a
                // destroyed mutex. Guest programming error; signaler
                // returns ESRCH but the cond waiter was already
                // dequeued, so drop them from the wake path and
                // log defensively by setting r3 = 0 via a plain
                // wake (they return OK with no mutex, letting the
                // guest's assertion catch the error).
                Lv2Dispatch::WakeAndReturn {
                    code: 0x8001_0005,
                    woken_unit_ids: vec![waker_unit],
                    response_updates: vec![(
                        waker_unit,
                        PendingResponse::ReturnCode { code: 0x8001_0005 },
                    )],
                    effects: vec![],
                }
            }
        }
    }

    fn dispatch_event_queue_tryreceive(
        &mut self,
        id: u32,
        event_array: u32,
        size: u32,
        count_out: u32,
        requester: UnitId,
    ) -> Lv2Dispatch {
        // Drain up to `size` payloads and write them to the
        // output array as consecutive 32-byte sys_event_t
        // structs. Write the actual count to count_out. Returns
        // CELL_OK regardless of how many payloads were available
        // (zero is a valid result).
        let Some(batch) = self.event_queues.try_receive_batch(id, size as usize) else {
            return Lv2Dispatch::Immediate {
                code: 0x8001_0005, // CELL_ESRCH
                effects: vec![],
            };
        };
        let count = batch.len() as u32;
        let mut effects: Vec<Effect> = Vec::new();
        for (i, payload) in batch.iter().enumerate() {
            let mut buf = [0u8; 32];
            buf[0..8].copy_from_slice(&payload.source.to_be_bytes());
            buf[8..16].copy_from_slice(&payload.data1.to_be_bytes());
            buf[16..24].copy_from_slice(&payload.data2.to_be_bytes());
            buf[24..32].copy_from_slice(&payload.data3.to_be_bytes());
            let addr = event_array as u64 + (i as u64) * 32;
            if let Some(range) = ByteRange::new(GuestAddr::new(addr), 32) {
                effects.push(Effect::SharedWriteIntent {
                    range,
                    bytes: WritePayload::from_slice(&buf),
                    ordering: PriorityClass::Normal,
                    source: requester,
                    source_time: GuestTicks::ZERO,
                });
            }
        }
        // Write the count to count_out.
        if let Some(range) = ByteRange::new(GuestAddr::new(count_out as u64), 4) {
            effects.push(Effect::SharedWriteIntent {
                range,
                bytes: WritePayload::from_slice(&count.to_be_bytes()),
                ordering: PriorityClass::Normal,
                source: requester,
                source_time: GuestTicks::ZERO,
            });
        }
        Lv2Dispatch::Immediate { code: 0, effects }
    }

    fn dispatch_event_port_send(
        &mut self,
        port_id: u32,
        data1: u64,
        data2: u64,
        data3: u64,
    ) -> Lv2Dispatch {
        // Port id is treated as queue id directly (1:1 binding);
        // the payload's `source` field carries the port id so the
        // receiver knows which port delivered the event.
        let payload = EventPayload {
            source: port_id as u64,
            data1,
            data2,
            data3,
        };
        match self.event_queues.send_and_wake_or_enqueue(port_id, payload) {
            crate::sync_primitives::EventQueueSend::Unknown => Lv2Dispatch::Immediate {
                code: 0x8001_0005,
                effects: vec![],
            },
            crate::sync_primitives::EventQueueSend::Full => Lv2Dispatch::Immediate {
                code: 0x8001_000A, // CELL_EBUSY
                effects: vec![],
            },
            crate::sync_primitives::EventQueueSend::Enqueued => Lv2Dispatch::Immediate {
                code: 0,
                effects: vec![],
            },
            crate::sync_primitives::EventQueueSend::Woke {
                new_owner,
                out_ptr,
                payload,
            } => {
                // Send_and_wake_or_enqueue returned the waiter's
                // recorded out_ptr alongside the thread id, so
                // the response_update carries a complete
                // PendingResponse -- no merge-with-parked-response
                // magic needed.
                if let Some(thread) = self.ppu_threads.get(new_owner) {
                    Lv2Dispatch::WakeAndReturn {
                        code: 0,
                        woken_unit_ids: vec![thread.unit_id],
                        response_updates: vec![(
                            thread.unit_id,
                            PendingResponse::EventQueueReceive {
                                out_ptr,
                                source: payload.source,
                                data1: payload.data1,
                                data2: payload.data2,
                                data3: payload.data3,
                            },
                        )],
                        effects: vec![],
                    }
                } else {
                    Lv2Dispatch::Immediate {
                        code: 0,
                        effects: vec![],
                    }
                }
            }
        }
    }

    fn dispatch_ppu_thread_join(
        &mut self,
        target: u64,
        status_out_ptr: u32,
        requester: UnitId,
    ) -> Lv2Dispatch {
        let target_id = PpuThreadId::new(target);
        // Resolve the target. If it does not exist in the table
        // the caller passed a bogus id: return CELL_ESRCH.
        let Some(target_thread) = self.ppu_threads.get(target_id) else {
            return Lv2Dispatch::Immediate {
                code: 0x8001_0005,
                effects: vec![],
            };
        };
        // If the target has already exited, resolve immediately:
        // write the exit value to status_out_ptr and return
        // CELL_OK. No block, no table update.
        if matches!(
            target_thread.state,
            crate::ppu_thread::PpuThreadState::Finished
        ) {
            let exit_value = target_thread.exit_value.unwrap_or(0);
            let write = Effect::SharedWriteIntent {
                range: ByteRange::new(GuestAddr::new(status_out_ptr as u64), 8).unwrap(),
                bytes: WritePayload::from_slice(&exit_value.to_be_bytes()),
                ordering: PriorityClass::Normal,
                source: requester,
                source_time: GuestTicks::ZERO,
            };
            return Lv2Dispatch::Immediate {
                code: 0,
                effects: vec![write],
            };
        }
        // Target is still Runnable / Blocked. Record the caller
        // on the target's join-waiter list and block the caller.
        // The caller's own guest thread id is needed as the
        // waiter key. If the requester is not a tracked PPU
        // thread (shouldn't happen in practice), fall back to a
        // sentinel id so the record still takes place.
        let caller_thread_id = self
            .ppu_threads
            .thread_id_for_unit(requester)
            .unwrap_or(PpuThreadId::PRIMARY);
        self.ppu_threads
            .add_join_waiter(target_id, caller_thread_id);
        Lv2Dispatch::Block {
            reason: crate::dispatch::Lv2BlockReason::PpuThreadJoin { target },
            pending: PendingResponse::PpuThreadJoin {
                target,
                status_out_ptr,
            },
            effects: vec![],
        }
    }

    fn dispatch_ppu_thread_create(
        &mut self,
        id_ptr: u32,
        entry_opd: u32,
        arg: u64,
        priority: u32,
        stacksize: u64,
    ) -> Lv2Dispatch {
        // Enforce a minimum stack size so the ABI-required
        // back-chain and register save area always fit. PSL1GHT
        // defaults to 0x10_000 (64 KB); callers may request
        // smaller or zero.
        let size = stacksize.max(0x4000);
        let stack = match self.allocate_child_stack(size, 0x10) {
            Some(s) => s,
            None => {
                // Stack-region exhaustion. Return ENOMEM-class
                // error. Real LV2 returns CELL_ENOMEM on stack
                // allocation failure.
                return Lv2Dispatch::Immediate {
                    code: 0x8001_0004,
                    effects: vec![],
                };
            }
        };

        // Instantiate a per-thread TLS block from the captured
        // template. Empty template yields an empty Vec, which
        // the runtime treats as "no TLS bytes to commit, r13=0".
        let tls_bytes = self.tls_template.instantiate();
        // Place the TLS block immediately above the child stack
        // so the layout is deterministic. If the template is
        // empty, tls_base is zero and the runtime skips the
        // TLS-commit effect.
        let tls_base = if tls_bytes.is_empty() {
            0
        } else {
            // Round up to 16-byte boundary above stack_top for
            // ABI alignment. The child's r13 points here.
            (stack.end() + 0xF) & !0xF
        };

        Lv2Dispatch::PpuThreadCreate {
            id_ptr,
            entry_opd,
            stack_top: stack.initial_sp(),
            stack_base: stack.base,
            stack_size: stack.size,
            arg,
            tls_base,
            tls_bytes,
            priority,
            effects: vec![],
        }
    }

    fn dispatch_ppu_thread_exit(&mut self, exit_value: u64, requester: UnitId) -> Lv2Dispatch {
        // Look up the calling thread's guest id in the table.
        // Foundation titles do not seed the primary thread yet,
        // so the table may not contain the caller; in that case
        // the runtime still needs to transition the caller to
        // Finished but no waiters exist to wake.
        let waiters_unit_ids = match self.ppu_threads.thread_id_for_unit(requester) {
            Some(tid) => {
                let waiter_thread_ids = self.ppu_threads.mark_finished(tid, exit_value);
                // Translate guest thread ids back to runtime
                // UnitIds. A waiter that has been detached or
                // purged between join-time and now is simply
                // skipped.
                waiter_thread_ids
                    .into_iter()
                    .filter_map(|wtid| self.ppu_threads.get(wtid).map(|t| t.unit_id))
                    .collect()
            }
            None => Vec::new(),
        };
        Lv2Dispatch::PpuThreadExit {
            exit_value,
            woken_unit_ids: waiters_unit_ids,
            effects: vec![],
        }
    }

    fn dispatch_memory_allocate(
        &mut self,
        size: u64,
        alloc_addr_ptr: u32,
        requester: UnitId,
    ) -> Lv2Dispatch {
        // Align to 64KB boundary.
        let align = 0x10000u32;
        let aligned_ptr = (self.mem_alloc_ptr + align - 1) & !(align - 1);
        self.mem_alloc_ptr = aligned_ptr + size as u32;
        Self::immediate_write_u32(aligned_ptr, alloc_addr_ptr, requester)
    }
}

#[cfg(test)]
#[path = "tests/host_tests.rs"]
mod tests;
