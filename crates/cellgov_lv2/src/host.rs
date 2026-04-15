//! `Lv2Host` -- the LV2 model the runtime calls into.
//!
//! The host owns image registry and thread group state. The runtime
//! calls `dispatch` once per syscall yield, synchronously, during the
//! same `step()` that processed the yield. The host reads guest memory
//! through the `Lv2Runtime` trait and returns an `Lv2Dispatch` telling
//! the runtime what to do.

use crate::dispatch::Lv2Dispatch;
use crate::image::ContentStore;
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
    /// Construct an empty host with no registered images or groups.
    pub fn new() -> Self {
        Self {
            content: ContentStore::new(),
            groups: ThreadGroupTable::new(),
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
}
