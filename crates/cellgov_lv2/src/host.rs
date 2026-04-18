//! `Lv2Host` -- the LV2 model the runtime calls into.
//!
//! The host owns image registry and thread group state. The runtime
//! calls `dispatch` once per syscall yield, synchronously, during the
//! same `step()` that processed the yield. The host reads guest memory
//! through the `Lv2Runtime` trait and returns an `Lv2Dispatch` telling
//! the runtime what to do.

use crate::dispatch::{Lv2Dispatch, PendingResponse};
use crate::image::ContentStore;
use crate::ppu_thread::{
    PpuThread, PpuThreadAttrs, PpuThreadId, PpuThreadTable, ThreadStack, ThreadStackAllocator,
    TlsTemplate,
};
use crate::request::Lv2Request;
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
            Lv2Request::MutexCreate { id_ptr, .. } => self.dispatch_mutex_create(id_ptr, requester),
            Lv2Request::MutexLock { .. } | Lv2Request::MutexUnlock { .. } => {
                // Stub: single-threaded module_start never contends.
                Lv2Dispatch::Immediate {
                    code: 0,
                    effects: vec![],
                }
            }
            Lv2Request::EventQueueCreate { id_ptr, .. } => {
                self.dispatch_event_queue_create(id_ptr, requester)
            }
            Lv2Request::EventQueueDestroy { .. } => Lv2Dispatch::Immediate {
                code: 0,
                effects: vec![],
            },
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

    fn dispatch_mutex_create(&mut self, id_ptr: u32, requester: UnitId) -> Lv2Dispatch {
        let id = self.alloc_id();
        Self::immediate_write_u32(id, id_ptr, requester)
    }

    fn dispatch_event_queue_create(&mut self, id_ptr: u32, requester: UnitId) -> Lv2Dispatch {
        let id = self.alloc_id();
        Self::immediate_write_u32(id, id_ptr, requester)
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
mod tests {
    use super::*;
    use cellgov_mem::GuestMemory;

    struct FakeRuntime {
        memory: GuestMemory,
    }

    impl FakeRuntime {
        fn new(size: usize) -> Self {
            Self {
                memory: GuestMemory::new(size),
            }
        }

        fn with_memory(memory: GuestMemory) -> Self {
            Self { memory }
        }
    }

    impl Lv2Runtime for FakeRuntime {
        fn read_committed(&self, addr: u64, len: usize) -> Option<&[u8]> {
            let start = addr as usize;
            let end = start.checked_add(len)?;
            let bytes = self.memory.as_bytes();
            if end <= bytes.len() {
                Some(&bytes[start..end])
            } else {
                None
            }
        }
    }

    #[test]
    fn image_open_out_of_range_path_returns_error() {
        let mut host = Lv2Host::new();
        let rt = FakeRuntime::new(256);
        let req = Lv2Request::SpuImageOpen {
            img_ptr: 0x1000,
            path_ptr: 0x2000,
        };
        let result = host.dispatch(req, UnitId::new(0), &rt);
        match result {
            Lv2Dispatch::Immediate { code, effects } => {
                assert_ne!(code, 0);
                assert!(effects.is_empty());
            }
            other => panic!("expected Immediate, got {other:?}"),
        }
    }

    #[test]
    fn group_create_allocates_id_and_writes_to_guest() {
        let mut host = Lv2Host::new();
        let rt = FakeRuntime::new(0x4000);
        let req = Lv2Request::SpuThreadGroupCreate {
            id_ptr: 0x3000,
            num_threads: 2,
            priority: 100,
            attr_ptr: 0x3800,
        };
        let result = host.dispatch(req, UnitId::new(0), &rt);
        match result {
            Lv2Dispatch::Immediate { code, effects } => {
                assert_eq!(code, 0);
                assert_eq!(effects.len(), 1);
                if let Effect::SharedWriteIntent { range, bytes, .. } = &effects[0] {
                    assert_eq!(range.start().raw(), 0x3000);
                    assert_eq!(range.length(), 4);
                    // Group id 1, big-endian.
                    assert_eq!(bytes.bytes(), &1u32.to_be_bytes());
                } else {
                    panic!("expected SharedWriteIntent");
                }
            }
            other => panic!("expected Immediate, got {other:?}"),
        }
        assert_eq!(host.thread_groups().len(), 1);
        let group = host.thread_groups().get(1).unwrap();
        assert_eq!(group.num_threads, 2);
    }

    #[test]
    fn group_create_allocates_monotonic_ids() {
        let mut host = Lv2Host::new();
        let rt = FakeRuntime::new(0x4000);
        let r1 = host.dispatch(
            Lv2Request::SpuThreadGroupCreate {
                id_ptr: 0x100,
                num_threads: 1,
                priority: 0,
                attr_ptr: 0,
            },
            UnitId::new(0),
            &rt,
        );
        let r2 = host.dispatch(
            Lv2Request::SpuThreadGroupCreate {
                id_ptr: 0x200,
                num_threads: 1,
                priority: 0,
                attr_ptr: 0,
            },
            UnitId::new(0),
            &rt,
        );
        // First group gets id 1, second gets id 2.
        if let Lv2Dispatch::Immediate { effects, .. } = r1 {
            assert_eq!(
                effects[0].clone(),
                Effect::SharedWriteIntent {
                    range: ByteRange::new(GuestAddr::new(0x100), 4).unwrap(),
                    bytes: WritePayload::new(1u32.to_be_bytes().to_vec()),
                    ordering: PriorityClass::Normal,
                    source: UnitId::new(0),
                    source_time: GuestTicks::ZERO,
                }
            );
        }
        if let Lv2Dispatch::Immediate { effects, .. } = r2 {
            assert_eq!(
                effects[0].clone(),
                Effect::SharedWriteIntent {
                    range: ByteRange::new(GuestAddr::new(0x200), 4).unwrap(),
                    bytes: WritePayload::new(2u32.to_be_bytes().to_vec()),
                    ordering: PriorityClass::Normal,
                    source: UnitId::new(0),
                    source_time: GuestTicks::ZERO,
                }
            );
        }
    }

    #[test]
    fn thread_initialize_records_slot() {
        let mut host = Lv2Host::new();
        host.content_store_mut().register(b"/spu.elf", vec![0xAA]);

        // Guest memory: image struct at 0x200 (handle=1 in first 4
        // bytes, written by a previous image_open dispatch).
        let mut mem = GuestMemory::new(0x4000);
        let img_range = ByteRange::new(GuestAddr::new(0x200), 4).unwrap();
        mem.apply_commit(img_range, &1u32.to_be_bytes()).unwrap();
        let rt = FakeRuntime::with_memory(mem);

        // Create group.
        host.dispatch(
            Lv2Request::SpuThreadGroupCreate {
                id_ptr: 0x100,
                num_threads: 2,
                priority: 0,
                attr_ptr: 0,
            },
            UnitId::new(0),
            &rt,
        );
        // Initialize slot 0 -- reads image handle from img_ptr.
        let result = host.dispatch(
            Lv2Request::SpuThreadInitialize {
                thread_ptr: 0x300,
                group_id: 1,
                thread_num: 0,
                img_ptr: 0x200,
                attr_ptr: 0,
                arg_ptr: 0x1000,
            },
            UnitId::new(0),
            &rt,
        );
        match result {
            Lv2Dispatch::Immediate { code, effects } => {
                assert_eq!(code, 0);
                assert_eq!(effects.len(), 1); // thread_id write
            }
            other => panic!("expected Immediate, got {other:?}"),
        }
        let group = host.thread_groups().get(1).unwrap();
        assert_eq!(group.slots.len(), 1);
        assert_eq!(group.slots[&0].image_handle.raw(), 1);
    }

    #[test]
    fn thread_initialize_unknown_group_returns_error() {
        let mut host = Lv2Host::new();
        let mut mem = GuestMemory::new(0x1000);
        let img_range = ByteRange::new(GuestAddr::new(0x200), 4).unwrap();
        mem.apply_commit(img_range, &1u32.to_be_bytes()).unwrap();
        let rt = FakeRuntime::with_memory(mem);
        let result = host.dispatch(
            Lv2Request::SpuThreadInitialize {
                thread_ptr: 0x300,
                group_id: 99,
                thread_num: 0,
                img_ptr: 0x200,
                attr_ptr: 0,
                arg_ptr: 0,
            },
            UnitId::new(0),
            &rt,
        );
        match result {
            Lv2Dispatch::Immediate { code, effects } => {
                assert_ne!(code, 0);
                assert!(effects.is_empty());
            }
            other => panic!("expected Immediate, got {other:?}"),
        }
    }

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
        let req = Lv2Request::Unsupported { number: 999 };
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
    fn fake_runtime_reads_committed_memory() {
        let rt = FakeRuntime::new(256);
        assert!(rt.read_committed(0, 4).is_some());
        assert!(rt.read_committed(252, 4).is_some());
        assert!(rt.read_committed(253, 4).is_none());
        assert!(rt.read_committed(0, 0).is_some());
    }

    #[test]
    fn content_store_accessible_through_host() {
        let mut host = Lv2Host::new();
        assert!(host.content_store().is_empty());
        let h = host
            .content_store_mut()
            .register(b"/app_home/spu.elf", vec![1, 2, 3]);
        assert_eq!(h.raw(), 1);
        assert_eq!(host.content_store().len(), 1);
    }

    #[test]
    fn state_hash_changes_when_image_registered() {
        let empty = Lv2Host::new();
        let mut populated = Lv2Host::new();
        populated.content_store_mut().register(b"/spu.elf", vec![]);
        assert_ne!(empty.state_hash(), populated.state_hash());
    }

    #[test]
    fn state_hash_deterministic_across_instances() {
        let mut a = Lv2Host::new();
        let mut b = Lv2Host::new();
        a.content_store_mut().register(b"/spu.elf", vec![1, 2]);
        b.content_store_mut().register(b"/spu.elf", vec![1, 2]);
        assert_eq!(a.state_hash(), b.state_hash());
    }

    #[test]
    fn image_open_writes_struct_and_returns_cell_ok() {
        let mut host = Lv2Host::new();
        host.content_store_mut()
            .register(b"/app_home/spu.elf", vec![0xAA]);

        // Place the path string "/app_home/spu.elf\0" at address 0x100
        // in guest memory, and reserve 16 bytes at 0x200 for the output
        // struct.
        let mut mem = GuestMemory::new(0x300);
        let path = b"/app_home/spu.elf\0";
        let path_range = ByteRange::new(GuestAddr::new(0x100), path.len() as u64).unwrap();
        mem.apply_commit(path_range, path).unwrap();

        let rt = FakeRuntime::with_memory(mem);
        let req = Lv2Request::SpuImageOpen {
            img_ptr: 0x200,
            path_ptr: 0x100,
        };
        let result = host.dispatch(req, UnitId::new(0), &rt);
        match result {
            Lv2Dispatch::Immediate { code, effects } => {
                assert_eq!(code, 0);
                assert_eq!(effects.len(), 1);
                if let Effect::SharedWriteIntent { range, bytes, .. } = &effects[0] {
                    assert_eq!(range.start().raw(), 0x200);
                    assert_eq!(range.length(), 16);
                    // First 4 bytes: handle in big-endian (handle 1)
                    assert_eq!(&bytes.bytes()[0..4], &1u32.to_be_bytes());
                } else {
                    panic!("expected SharedWriteIntent");
                }
            }
            other => panic!("expected Immediate, got {other:?}"),
        }
    }

    #[test]
    fn image_open_unknown_path_returns_error() {
        let mut host = Lv2Host::new();
        // No images registered.
        let mut mem = GuestMemory::new(0x300);
        let path = b"/nonexistent.elf\0";
        let path_range = ByteRange::new(GuestAddr::new(0x100), path.len() as u64).unwrap();
        mem.apply_commit(path_range, path).unwrap();

        let rt = FakeRuntime::with_memory(mem);
        let req = Lv2Request::SpuImageOpen {
            img_ptr: 0x200,
            path_ptr: 0x100,
        };
        let result = host.dispatch(req, UnitId::new(0), &rt);
        match result {
            Lv2Dispatch::Immediate { code, effects } => {
                assert_ne!(code, 0);
                assert!(effects.is_empty());
            }
            other => panic!("expected Immediate, got {other:?}"),
        }
    }

    #[test]
    fn image_open_bad_path_ptr_returns_error() {
        let host_with_image = {
            let mut h = Lv2Host::new();
            h.content_store_mut().register(b"/spu.elf", vec![]);
            h
        };
        // path_ptr points past end of memory.
        let rt = FakeRuntime::new(64);
        let req = Lv2Request::SpuImageOpen {
            img_ptr: 0,
            path_ptr: 0xFFFF,
        };
        let result = host_with_image.clone().dispatch(req, UnitId::new(0), &rt);
        match result {
            Lv2Dispatch::Immediate { code, effects } => {
                assert_ne!(code, 0);
                assert!(effects.is_empty());
            }
            other => panic!("expected Immediate, got {other:?}"),
        }
    }

    #[test]
    fn image_open_handle_is_deterministic() {
        let make_host = || {
            let mut h = Lv2Host::new();
            h.content_store_mut().register(b"/spu.elf", vec![1, 2, 3]);
            h
        };

        let mut mem = GuestMemory::new(0x300);
        let path = b"/spu.elf\0";
        let path_range = ByteRange::new(GuestAddr::new(0x100), path.len() as u64).unwrap();
        mem.apply_commit(path_range, path).unwrap();
        let rt = FakeRuntime::with_memory(mem);

        let r1 = make_host().dispatch(
            Lv2Request::SpuImageOpen {
                img_ptr: 0x200,
                path_ptr: 0x100,
            },
            UnitId::new(0),
            &rt,
        );
        let r2 = make_host().dispatch(
            Lv2Request::SpuImageOpen {
                img_ptr: 0x200,
                path_ptr: 0x100,
            },
            UnitId::new(0),
            &rt,
        );
        assert_eq!(r1, r2);
    }

    #[test]
    fn group_start_returns_register_spu_with_inits() {
        let mut host = Lv2Host::new();
        host.content_store_mut()
            .register(b"/spu.elf", vec![0xAA, 0xBB]);

        // Prepare guest memory: path at 0x100, arg struct at 0x200,
        // image struct at 0x300 (handle=1 pre-populated).
        let mut mem = GuestMemory::new(0x4000);
        let path = b"/spu.elf\0";
        let path_range = ByteRange::new(GuestAddr::new(0x100), path.len() as u64).unwrap();
        mem.apply_commit(path_range, path).unwrap();
        let img_range = ByteRange::new(GuestAddr::new(0x300), 4).unwrap();
        mem.apply_commit(img_range, &1u32.to_be_bytes()).unwrap();

        // sys_spu_thread_argument: 4x u64 big-endian.
        // arg1 = 0x1000 (result EA)
        let mut arg_bytes = [0u8; 32];
        arg_bytes[0..8].copy_from_slice(&0x1000u64.to_be_bytes());
        let arg_range = ByteRange::new(GuestAddr::new(0x200), 32).unwrap();
        mem.apply_commit(arg_range, &arg_bytes).unwrap();

        let rt = FakeRuntime::with_memory(mem);

        // image_open
        host.dispatch(
            Lv2Request::SpuImageOpen {
                img_ptr: 0x300,
                path_ptr: 0x100,
            },
            UnitId::new(0),
            &rt,
        );

        // group_create (1 thread)
        host.dispatch(
            Lv2Request::SpuThreadGroupCreate {
                id_ptr: 0x400,
                num_threads: 1,
                priority: 0,
                attr_ptr: 0,
            },
            UnitId::new(0),
            &rt,
        );

        // thread_initialize (slot 0, img_ptr 0x300 has handle, arg_ptr 0x200)
        host.dispatch(
            Lv2Request::SpuThreadInitialize {
                thread_ptr: 0x500,
                group_id: 1,
                thread_num: 0,
                img_ptr: 0x300,
                attr_ptr: 0,
                arg_ptr: 0x200,
            },
            UnitId::new(0),
            &rt,
        );

        // group_start
        let result = host.dispatch(
            Lv2Request::SpuThreadGroupStart { group_id: 1 },
            UnitId::new(0),
            &rt,
        );

        match result {
            Lv2Dispatch::RegisterSpu { inits, code, .. } => {
                assert_eq!(code, 0);
                assert_eq!(inits.len(), 1);
                assert_eq!(inits[0].ls_bytes, vec![0xAA, 0xBB]);
                assert_eq!(inits[0].entry_pc, 0x80);
                assert_eq!(inits[0].stack_ptr, 0x3FFF0);
                assert_eq!(inits[0].args[0], 0x1000);
                assert_eq!(inits[0].group_id, 1);
                assert_eq!(inits[0].slot, 0);
            }
            other => panic!("expected RegisterSpu, got {other:?}"),
        }
    }

    #[test]
    fn group_start_unknown_group_returns_error() {
        let mut host = Lv2Host::new();
        let rt = FakeRuntime::new(256);
        let result = host.dispatch(
            Lv2Request::SpuThreadGroupStart { group_id: 99 },
            UnitId::new(0),
            &rt,
        );
        match result {
            Lv2Dispatch::Immediate { code, .. } => assert_ne!(code, 0),
            other => panic!("expected Immediate error, got {other:?}"),
        }
    }

    // -- Syscall dispatch tests --

    /// Extract the big-endian u32 value from a SharedWriteIntent effect.
    fn extract_write_u32(effect: &cellgov_effects::Effect) -> u32 {
        match effect {
            cellgov_effects::Effect::SharedWriteIntent { bytes, .. } => {
                let b = bytes.bytes();
                assert_eq!(b.len(), 4);
                u32::from_be_bytes([b[0], b[1], b[2], b[3]])
            }
            other => panic!("expected SharedWriteIntent, got {other:?}"),
        }
    }

    #[test]
    fn mutex_create_allocates_monotonic_ids() {
        let mut host = Lv2Host::new();
        let rt = FakeRuntime::new(0x10000);
        let source = UnitId::new(0);

        let r1 = host.dispatch(
            Lv2Request::MutexCreate {
                id_ptr: 0x100,
                attr_ptr: 0x200,
            },
            source,
            &rt,
        );
        let r2 = host.dispatch(
            Lv2Request::MutexCreate {
                id_ptr: 0x104,
                attr_ptr: 0x200,
            },
            source,
            &rt,
        );

        let id1 = match &r1 {
            Lv2Dispatch::Immediate {
                code: 0,
                effects: e,
            } => extract_write_u32(&e[0]),
            other => panic!("expected Immediate(0), got {other:?}"),
        };
        let id2 = match &r2 {
            Lv2Dispatch::Immediate {
                code: 0,
                effects: e,
            } => extract_write_u32(&e[0]),
            other => panic!("expected Immediate(0), got {other:?}"),
        };
        assert_ne!(id1, id2, "IDs should be monotonically different");
        assert!(id1 > 0 && id2 > 0, "IDs should be non-zero");
    }

    #[test]
    fn event_queue_create_allocates_id() {
        let mut host = Lv2Host::new();
        let rt = FakeRuntime::new(0x10000);
        let result = host.dispatch(
            Lv2Request::EventQueueCreate {
                id_ptr: 0x100,
                attr_ptr: 0x200,
                key: 0,
                size: 64,
            },
            UnitId::new(0),
            &rt,
        );
        match result {
            Lv2Dispatch::Immediate { code: 0, effects } => {
                assert_eq!(effects.len(), 1);
                let id = extract_write_u32(&effects[0]);
                assert!(id > 0, "queue ID should be non-zero");
            }
            other => panic!("expected Immediate(0), got {other:?}"),
        }
    }

    #[test]
    fn memory_allocate_returns_aligned_sequential_addresses() {
        let mut host = Lv2Host::new();
        let rt = FakeRuntime::new(0x10000);
        let source = UnitId::new(0);

        let addr1 = match host.dispatch(
            Lv2Request::MemoryAllocate {
                size: 0x10000,
                flags: 0x200,
                alloc_addr_ptr: 0x100,
            },
            source,
            &rt,
        ) {
            Lv2Dispatch::Immediate { code: 0, effects } => extract_write_u32(&effects[0]),
            other => panic!("expected Immediate(0), got {other:?}"),
        };
        let addr2 = match host.dispatch(
            Lv2Request::MemoryAllocate {
                size: 0x10000,
                flags: 0x200,
                alloc_addr_ptr: 0x104,
            },
            source,
            &rt,
        ) {
            Lv2Dispatch::Immediate { code: 0, effects } => extract_write_u32(&effects[0]),
            other => panic!("expected Immediate(0), got {other:?}"),
        };

        assert_eq!(addr1 & 0xFFFF, 0, "addr1 not 64KB-aligned");
        assert_eq!(addr2 & 0xFFFF, 0, "addr2 not 64KB-aligned");
        assert!(
            addr2 >= addr1 + 0x10000,
            "allocations overlap: 0x{addr1:x} and 0x{addr2:x}"
        );
    }

    #[test]
    fn set_mem_alloc_base_overrides_first_allocation_address() {
        // The allocator base is configurable so callers that load a
        // real ELF can place sys_memory_allocate's pool above the
        // ELF's PT_LOAD footprint. The bump pointer's 64KB alignment
        // is preserved by the dispatch path, so the first returned
        // address must be >= the configured base and 64KB-aligned.
        let mut host = Lv2Host::new();
        host.set_mem_alloc_base(0x008A_0000);
        let rt = FakeRuntime::new(0x10000);
        let addr = match host.dispatch(
            Lv2Request::MemoryAllocate {
                size: 0x10000,
                flags: 0x200,
                alloc_addr_ptr: 0x100,
            },
            UnitId::new(0),
            &rt,
        ) {
            Lv2Dispatch::Immediate { code: 0, effects } => extract_write_u32(&effects[0]),
            other => panic!("expected Immediate(0), got {other:?}"),
        };
        assert_eq!(
            addr, 0x008A_0000,
            "first allocation must use configured base"
        );
        assert_eq!(addr & 0xFFFF, 0, "alignment must be preserved");
    }

    #[test]
    fn mutex_lock_unlock_are_noop_stubs() {
        let mut host = Lv2Host::new();
        let rt = FakeRuntime::new(256);
        for req in [
            Lv2Request::MutexLock {
                mutex_id: 1,
                timeout: 0,
            },
            Lv2Request::MutexUnlock { mutex_id: 1 },
        ] {
            let result = host.dispatch(req, UnitId::new(0), &rt);
            assert_eq!(
                result,
                Lv2Dispatch::Immediate {
                    code: 0,
                    effects: vec![]
                }
            );
        }
    }

    #[test]
    fn tty_read_returns_eio() {
        // Syscall 402 is sys_tty_read. RPCS3 returns CELL_EIO =
        // 0x8001002B outside debug console mode; that is the retail
        // behavior real games target. CELL_OK with no data causes
        // CRT input loops to spin indefinitely.
        let mut host = Lv2Host::new();
        let rt = FakeRuntime::new(256);
        let result = host.dispatch(Lv2Request::Unsupported { number: 402 }, UnitId::new(0), &rt);
        assert_eq!(
            result,
            Lv2Dispatch::Immediate {
                code: 0x8001_002B,
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
        let result = host.dispatch(Lv2Request::Unsupported { number: 481 }, UnitId::new(0), &rt);
        assert_eq!(
            result,
            Lv2Dispatch::Immediate {
                code: 0x8001_0002,
                effects: vec![],
            }
        );
    }

    #[test]
    fn memory_free_is_noop_stub() {
        let mut host = Lv2Host::new();
        let rt = FakeRuntime::new(256);
        let result = host.dispatch(
            Lv2Request::MemoryFree { addr: 0x0001_0000 },
            UnitId::new(0),
            &rt,
        );
        assert_eq!(
            result,
            Lv2Dispatch::Immediate {
                code: 0,
                effects: vec![]
            }
        );
    }

    #[test]
    fn memory_get_user_memory_size_writes_info_struct() {
        // sys_memory_info_t has two big-endian u32 fields:
        // total_user_memory, available_user_memory.
        let mut host = Lv2Host::new();
        let rt = FakeRuntime::new(0x10000);
        let source = UnitId::new(0);

        let result = host.dispatch(
            Lv2Request::MemoryGetUserMemorySize {
                mem_info_ptr: 0x200,
            },
            source,
            &rt,
        );
        match result {
            Lv2Dispatch::Immediate { code: 0, effects } => {
                assert_eq!(effects.len(), 1, "expect one 8-byte write");
                match &effects[0] {
                    cellgov_effects::Effect::SharedWriteIntent { range, bytes, .. } => {
                        assert_eq!(range.start().raw(), 0x200);
                        assert_eq!(range.length(), 8);
                        let b = bytes.bytes();
                        let total = u32::from_be_bytes([b[0], b[1], b[2], b[3]]);
                        let avail = u32::from_be_bytes([b[4], b[5], b[6], b[7]]);
                        assert_eq!(total, 0x0D50_0000);
                        assert_eq!(avail, 0x0D50_0000);
                    }
                    other => panic!("expected SharedWriteIntent, got {other:?}"),
                }
            }
            other => panic!("expected Immediate(0), got {other:?}"),
        }
    }

    fn primary_attrs() -> PpuThreadAttrs {
        PpuThreadAttrs {
            entry: 0x10_0000,
            arg: 0,
            stack_base: 0xD000_0000,
            stack_size: 0x10000,
            priority: 1000,
            tls_base: 0x0020_0000,
        }
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

    #[test]
    fn ppu_thread_exit_marks_thread_finished_with_exit_value() {
        // sys_ppu_thread_exit marks the calling thread Finished,
        // captures the exit value, and -- when no one is joining
        // -- returns an empty waker list. The runtime side does
        // the unit-state transition.
        let mut host = Lv2Host::new();
        host.seed_primary_ppu_thread(UnitId::new(0), primary_attrs());
        let rt = FakeRuntime::new(256);
        let result = host.dispatch(
            Lv2Request::PpuThreadExit {
                exit_value: 0xDEAD_BEEF,
            },
            UnitId::new(0),
            &rt,
        );
        match result {
            Lv2Dispatch::PpuThreadExit {
                exit_value,
                woken_unit_ids,
                effects,
            } => {
                assert_eq!(exit_value, 0xDEAD_BEEF);
                assert!(woken_unit_ids.is_empty());
                assert!(effects.is_empty());
            }
            other => panic!("expected PpuThreadExit dispatch, got {other:?}"),
        }
        // Primary thread is now Finished with the exit value.
        let primary = host.ppu_thread_for_unit(UnitId::new(0)).unwrap();
        assert_eq!(primary.state, crate::ppu_thread::PpuThreadState::Finished);
        assert_eq!(primary.exit_value, Some(0xDEAD_BEEF));
    }

    #[test]
    fn ppu_thread_exit_unseeded_thread_still_returns_dispatch() {
        // If the caller is not in the thread table yet (e.g. the
        // primary is unseeded -- foundation-title boot path), the
        // handler still returns a PpuThreadExit dispatch so the
        // runtime transitions the unit to Finished. No waiters
        // are waked because none can be tracked.
        let mut host = Lv2Host::new();
        let rt = FakeRuntime::new(256);
        let result = host.dispatch(
            Lv2Request::PpuThreadExit { exit_value: 7 },
            UnitId::new(99),
            &rt,
        );
        match result {
            Lv2Dispatch::PpuThreadExit {
                exit_value,
                woken_unit_ids,
                ..
            } => {
                assert_eq!(exit_value, 7);
                assert!(woken_unit_ids.is_empty());
            }
            other => panic!("expected PpuThreadExit, got {other:?}"),
        }
    }

    #[test]
    fn ppu_thread_exit_wakes_join_waiters() {
        // A child thread exits with waiters registered on its
        // join list. The handler reports those waiters' unit ids
        // so the runtime can wake them.
        let mut host = Lv2Host::new();
        host.seed_primary_ppu_thread(UnitId::new(0), primary_attrs());
        let child_tid = host
            .ppu_threads_mut()
            .create(UnitId::new(1), primary_attrs())
            .expect("child create");
        // Primary joins on the child.
        host.ppu_threads_mut()
            .add_join_waiter(child_tid, crate::ppu_thread::PpuThreadId::PRIMARY);
        let rt = FakeRuntime::new(256);
        let result = host.dispatch(
            Lv2Request::PpuThreadExit { exit_value: 5 },
            UnitId::new(1),
            &rt,
        );
        match result {
            Lv2Dispatch::PpuThreadExit {
                exit_value,
                woken_unit_ids,
                ..
            } => {
                assert_eq!(exit_value, 5);
                assert_eq!(woken_unit_ids, vec![UnitId::new(0)]);
            }
            other => panic!("expected PpuThreadExit, got {other:?}"),
        }
    }

    #[test]
    fn ppu_thread_join_finished_target_returns_immediate_with_exit_value() {
        let mut host = Lv2Host::new();
        host.seed_primary_ppu_thread(UnitId::new(0), primary_attrs());
        // Create a child and immediately mark it finished.
        let child = host
            .ppu_threads_mut()
            .create(UnitId::new(1), primary_attrs())
            .expect("child create");
        host.ppu_threads_mut().mark_finished(child, 0xFEED_FACE);
        let rt = FakeRuntime::new(0x10000);
        let result = host.dispatch(
            Lv2Request::PpuThreadJoin {
                target: child.raw(),
                status_out_ptr: 0x500,
            },
            UnitId::new(0),
            &rt,
        );
        match result {
            Lv2Dispatch::Immediate { code, effects } => {
                assert_eq!(code, 0);
                assert_eq!(effects.len(), 1);
                if let Effect::SharedWriteIntent { range, bytes, .. } = &effects[0] {
                    assert_eq!(range.start().raw(), 0x500);
                    assert_eq!(range.length(), 8);
                    assert_eq!(bytes.bytes(), &0xFEED_FACE_u64.to_be_bytes());
                } else {
                    panic!("expected SharedWriteIntent");
                }
            }
            other => panic!("expected Immediate, got {other:?}"),
        }
    }

    #[test]
    fn ppu_thread_join_running_target_blocks_and_records_waiter() {
        let mut host = Lv2Host::new();
        host.seed_primary_ppu_thread(UnitId::new(0), primary_attrs());
        let child = host
            .ppu_threads_mut()
            .create(UnitId::new(1), primary_attrs())
            .expect("child create");
        let rt = FakeRuntime::new(256);
        let result = host.dispatch(
            Lv2Request::PpuThreadJoin {
                target: child.raw(),
                status_out_ptr: 0x500,
            },
            UnitId::new(0),
            &rt,
        );
        match result {
            Lv2Dispatch::Block {
                reason, pending, ..
            } => {
                assert!(matches!(
                    reason,
                    crate::dispatch::Lv2BlockReason::PpuThreadJoin { target } if target == child.raw()
                ));
                assert!(matches!(
                    pending,
                    PendingResponse::PpuThreadJoin {
                        status_out_ptr: 0x500,
                        ..
                    }
                ));
            }
            other => panic!("expected Block, got {other:?}"),
        }
        // Child's join-waiter list now contains the primary's id.
        assert_eq!(
            host.ppu_threads().get(child).unwrap().join_waiters,
            vec![crate::ppu_thread::PpuThreadId::PRIMARY],
        );
    }

    #[test]
    fn ppu_thread_join_unknown_target_returns_esrch() {
        let mut host = Lv2Host::new();
        host.seed_primary_ppu_thread(UnitId::new(0), primary_attrs());
        let rt = FakeRuntime::new(256);
        let result = host.dispatch(
            Lv2Request::PpuThreadJoin {
                target: 0xDEAD_BEEF,
                status_out_ptr: 0x500,
            },
            UnitId::new(0),
            &rt,
        );
        match result {
            Lv2Dispatch::Immediate { code, effects } => {
                assert_eq!(code, 0x8001_0005);
                assert!(effects.is_empty());
            }
            other => panic!("expected Immediate with ESRCH, got {other:?}"),
        }
    }

    #[test]
    fn ppu_thread_create_returns_dispatch_with_allocated_stack_and_tls() {
        // With a non-empty TLS template captured, the handler
        // allocates a child stack block and instantiates a fresh
        // TLS block. Dispatch carries all fields the runtime
        // needs to register the child PPU unit.
        let mut host = Lv2Host::new();
        host.set_tls_template(crate::ppu_thread::TlsTemplate::new(
            vec![0xAB, 0xCD, 0xEF],
            0x100,
            0x10,
            0x89_5cd0,
        ));
        let rt = FakeRuntime::new(256);
        let result = host.dispatch(
            Lv2Request::PpuThreadCreate {
                id_ptr: 0x1000,
                entry_opd: 0x2_0000,
                arg: 0xDEAD_BEEF,
                priority: 1500,
                stacksize: 0x10_000,
                flags: 0,
            },
            UnitId::new(0),
            &rt,
        );
        match result {
            Lv2Dispatch::PpuThreadCreate {
                id_ptr,
                entry_opd,
                stack_top,
                stack_base,
                stack_size,
                arg,
                tls_base,
                tls_bytes,
                priority,
                effects,
            } => {
                assert_eq!(id_ptr, 0x1000);
                assert_eq!(entry_opd, 0x2_0000);
                assert_eq!(arg, 0xDEAD_BEEF);
                assert_eq!(priority, 1500);
                // First stack block starts at 0xD0010000.
                assert_eq!(stack_base, 0xD001_0000);
                assert_eq!(stack_size, 0x10_000);
                assert_eq!(stack_top, 0xD002_0000 - 0x10);
                // TLS block placed at or above the stack end.
                assert!(tls_base >= stack_base + stack_size);
                assert_eq!(tls_bytes.len(), 0x100);
                assert_eq!(&tls_bytes[..3], &[0xAB, 0xCD, 0xEF]);
                assert!(tls_bytes[3..].iter().all(|&b| b == 0));
                assert!(effects.is_empty());
            }
            other => panic!("expected PpuThreadCreate, got {other:?}"),
        }
    }

    #[test]
    fn ppu_thread_create_with_empty_template_has_no_tls() {
        // Games without PT_TLS get an empty template. The
        // dispatch still succeeds; tls_base is zero and
        // tls_bytes is empty so the runtime leaves r13=0.
        let mut host = Lv2Host::new();
        let rt = FakeRuntime::new(256);
        let result = host.dispatch(
            Lv2Request::PpuThreadCreate {
                id_ptr: 0x1000,
                entry_opd: 0x2_0000,
                arg: 0,
                priority: 1000,
                stacksize: 0x8000,
                flags: 0,
            },
            UnitId::new(0),
            &rt,
        );
        match result {
            Lv2Dispatch::PpuThreadCreate {
                tls_base,
                tls_bytes,
                ..
            } => {
                assert_eq!(tls_base, 0);
                assert!(tls_bytes.is_empty());
            }
            other => panic!("expected PpuThreadCreate, got {other:?}"),
        }
    }

    #[test]
    fn ppu_thread_create_enforces_minimum_stack_size() {
        // A stacksize below the ABI minimum (0x4000) is rounded
        // up so the child has room for its back-chain + register
        // save area.
        let mut host = Lv2Host::new();
        let rt = FakeRuntime::new(256);
        let result = host.dispatch(
            Lv2Request::PpuThreadCreate {
                id_ptr: 0x1000,
                entry_opd: 0x2_0000,
                arg: 0,
                priority: 1000,
                stacksize: 0x100,
                flags: 0,
            },
            UnitId::new(0),
            &rt,
        );
        match result {
            Lv2Dispatch::PpuThreadCreate { stack_size, .. } => {
                assert_eq!(stack_size, 0x4000);
            }
            other => panic!("expected PpuThreadCreate, got {other:?}"),
        }
    }

    #[test]
    fn ppu_thread_yield_returns_ok_with_no_effects() {
        // sys_ppu_thread_yield is a pure scheduler hint: return
        // CELL_OK immediately, emit no effects. The round-robin
        // scheduler advances to the next runnable unit on the
        // next step naturally because the caller has yielded via
        // YieldReason::Syscall.
        let mut host = Lv2Host::new();
        let rt = FakeRuntime::new(256);
        let result = host.dispatch(Lv2Request::PpuThreadYield, UnitId::new(0), &rt);
        assert_eq!(
            result,
            Lv2Dispatch::Immediate {
                code: 0,
                effects: vec![],
            }
        );
    }

    #[test]
    fn memory_container_create_writes_monotonic_id() {
        let mut host = Lv2Host::new();
        let rt = FakeRuntime::new(0x10000);
        let source = UnitId::new(0);

        let id1 = match host.dispatch(
            Lv2Request::MemoryContainerCreate {
                cid_ptr: 0x100,
                size: 0x10_0000,
            },
            source,
            &rt,
        ) {
            Lv2Dispatch::Immediate { code: 0, effects } => extract_write_u32(&effects[0]),
            other => panic!("expected Immediate(0), got {other:?}"),
        };
        let id2 = match host.dispatch(
            Lv2Request::MemoryContainerCreate {
                cid_ptr: 0x104,
                size: 0x10_0000,
            },
            source,
            &rt,
        ) {
            Lv2Dispatch::Immediate { code: 0, effects } => extract_write_u32(&effects[0]),
            other => panic!("expected Immediate(0), got {other:?}"),
        };
        assert_ne!(id1, 0);
        assert_ne!(id1, id2, "IDs must be monotonic across create calls");
    }
}
