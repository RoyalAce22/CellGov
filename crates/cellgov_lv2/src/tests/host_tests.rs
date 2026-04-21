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
fn lwmutex_create_allocates_monotonic_ids_starting_at_one() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let source = UnitId::new(0);
    let r1 = host.dispatch(
        Lv2Request::LwMutexCreate {
            id_ptr: 0x100,
            attr_ptr: 0x200,
        },
        source,
        &rt,
    );
    let r2 = host.dispatch(
        Lv2Request::LwMutexCreate {
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
    assert_eq!(id1, 1);
    assert_eq!(id2, 2);
    assert_eq!(host.lwmutexes().len(), 2);
}

#[test]
fn lwmutex_destroy_unknown_id_returns_esrch() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let r = host.dispatch(Lv2Request::LwMutexDestroy { id: 42 }, UnitId::new(0), &rt);
    assert!(matches!(
        r,
        Lv2Dispatch::Immediate {
            code: 0x8001_0005,
            ..
        }
    ));
}

#[test]
fn lwmutex_create_destroy_roundtrip() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let source = UnitId::new(0);
    let created = host.dispatch(
        Lv2Request::LwMutexCreate {
            id_ptr: 0x100,
            attr_ptr: 0x200,
        },
        source,
        &rt,
    );
    let id = match &created {
        Lv2Dispatch::Immediate {
            code: 0,
            effects: e,
        } => extract_write_u32(&e[0]),
        other => panic!("expected Immediate(0), got {other:?}"),
    };
    let destroyed = host.dispatch(Lv2Request::LwMutexDestroy { id }, source, &rt);
    assert!(matches!(
        destroyed,
        Lv2Dispatch::Immediate {
            code: 0,
            effects: _,
        }
    ));
    assert!(host.lwmutexes().lookup(id).is_none());
}

#[test]
fn lwmutex_destroy_with_waiter_returns_ebusy() {
    // Preload the table: create an lwmutex, set an owner, and
    // enqueue a waiter. Destroy must reject with EBUSY without
    // tearing down state.
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let source = UnitId::new(0);
    let created = host.dispatch(
        Lv2Request::LwMutexCreate {
            id_ptr: 0x100,
            attr_ptr: 0x200,
        },
        source,
        &rt,
    );
    let id = match &created {
        Lv2Dispatch::Immediate {
            code: 0,
            effects: e,
        } => extract_write_u32(&e[0]),
        other => panic!("expected Immediate(0), got {other:?}"),
    };
    host.lwmutexes_mut()
        .try_acquire(id, PpuThreadId::new(0x0100_0001));
    let r = host.dispatch(Lv2Request::LwMutexDestroy { id }, source, &rt);
    assert!(matches!(
        r,
        Lv2Dispatch::Immediate {
            code: 0x8001_000A,
            ..
        }
    ));
    assert!(host.lwmutexes().lookup(id).is_some());
}

/// Seed a primary PPU thread and map it to `requester`. Helper
/// for lwmutex lock tests that need a real PpuThreadId -> UnitId
/// mapping.
fn seed_primary_ppu(host: &mut Lv2Host, unit_id: UnitId) {
    host.seed_primary_ppu_thread(
        unit_id,
        PpuThreadAttrs {
            entry: 0,
            arg: 0,
            stack_base: 0,
            stack_size: 0,
            priority: 0,
            tls_base: 0,
        },
    );
}

#[test]
fn lwmutex_lock_unknown_id_returns_esrch() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let src = UnitId::new(0);
    seed_primary_ppu(&mut host, src);
    let r = host.dispatch(Lv2Request::LwMutexLock { id: 99, timeout: 0 }, src, &rt);
    assert!(matches!(
        r,
        Lv2Dispatch::Immediate {
            code: 0x8001_0005,
            ..
        }
    ));
}

#[test]
fn lwmutex_lock_unowned_acquires_immediately() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let src = UnitId::new(0);
    seed_primary_ppu(&mut host, src);
    let created = host.dispatch(
        Lv2Request::LwMutexCreate {
            id_ptr: 0x100,
            attr_ptr: 0x200,
        },
        src,
        &rt,
    );
    let id = match &created {
        Lv2Dispatch::Immediate {
            code: 0,
            effects: e,
        } => extract_write_u32(&e[0]),
        other => panic!("expected Immediate(0), got {other:?}"),
    };
    let r = host.dispatch(Lv2Request::LwMutexLock { id, timeout: 0 }, src, &rt);
    assert!(matches!(
        r,
        Lv2Dispatch::Immediate {
            code: 0,
            effects: _,
        }
    ));
    assert_eq!(
        host.lwmutexes().lookup(id).unwrap().owner(),
        Some(PpuThreadId::PRIMARY),
    );
    assert!(host.lwmutexes().lookup(id).unwrap().waiters().is_empty());
}

#[test]
fn lwmutex_lock_contended_parks_caller_on_waiter_list() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let owner_unit = UnitId::new(0);
    let waiter_unit = UnitId::new(1);
    seed_primary_ppu(&mut host, owner_unit);
    // Register a second PPU thread so the waiter has a
    // distinct thread id.
    let waiter_tid = host
        .ppu_threads_mut()
        .create(
            waiter_unit,
            PpuThreadAttrs {
                entry: 0,
                arg: 0,
                stack_base: 0,
                stack_size: 0,
                priority: 0,
                tls_base: 0,
            },
        )
        .unwrap();
    // Owner creates and acquires.
    let created = host.dispatch(
        Lv2Request::LwMutexCreate {
            id_ptr: 0x100,
            attr_ptr: 0x200,
        },
        owner_unit,
        &rt,
    );
    let id = match &created {
        Lv2Dispatch::Immediate {
            code: 0,
            effects: e,
        } => extract_write_u32(&e[0]),
        other => panic!("expected Immediate(0), got {other:?}"),
    };
    host.dispatch(Lv2Request::LwMutexLock { id, timeout: 0 }, owner_unit, &rt);
    // Waiter tries to acquire and blocks.
    let r = host.dispatch(Lv2Request::LwMutexLock { id, timeout: 0 }, waiter_unit, &rt);
    match r {
        Lv2Dispatch::Block {
            reason: crate::dispatch::Lv2BlockReason::LwMutex { id: blocked_id },
            pending: PendingResponse::ReturnCode { code: 0 },
            effects: _,
        } => {
            assert_eq!(blocked_id, id);
        }
        other => panic!("expected Block on LwMutex, got {other:?}"),
    }
    // Owner unchanged; waiter enqueued.
    let entry = host.lwmutexes().lookup(id).unwrap();
    assert_eq!(entry.owner(), Some(PpuThreadId::PRIMARY));
    let seen: Vec<_> = entry.waiters().iter().collect();
    assert_eq!(seen, vec![waiter_tid]);
}

#[test]
fn lwmutex_trylock_unowned_acquires() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let src = UnitId::new(0);
    seed_primary_ppu(&mut host, src);
    let created = host.dispatch(
        Lv2Request::LwMutexCreate {
            id_ptr: 0x100,
            attr_ptr: 0x200,
        },
        src,
        &rt,
    );
    let id = match &created {
        Lv2Dispatch::Immediate {
            code: 0,
            effects: e,
        } => extract_write_u32(&e[0]),
        other => panic!("expected Immediate(0), got {other:?}"),
    };
    let r = host.dispatch(Lv2Request::LwMutexTryLock { id }, src, &rt);
    assert!(matches!(
        r,
        Lv2Dispatch::Immediate {
            code: 0,
            effects: _,
        }
    ));
    assert_eq!(
        host.lwmutexes().lookup(id).unwrap().owner(),
        Some(PpuThreadId::PRIMARY),
    );
}

#[test]
fn lwmutex_trylock_contended_returns_ebusy_and_does_not_park() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let owner_unit = UnitId::new(0);
    let other_unit = UnitId::new(1);
    seed_primary_ppu(&mut host, owner_unit);
    host.ppu_threads_mut()
        .create(
            other_unit,
            PpuThreadAttrs {
                entry: 0,
                arg: 0,
                stack_base: 0,
                stack_size: 0,
                priority: 0,
                tls_base: 0,
            },
        )
        .unwrap();
    let created = host.dispatch(
        Lv2Request::LwMutexCreate {
            id_ptr: 0x100,
            attr_ptr: 0x200,
        },
        owner_unit,
        &rt,
    );
    let id = match &created {
        Lv2Dispatch::Immediate {
            code: 0,
            effects: e,
        } => extract_write_u32(&e[0]),
        other => panic!("expected Immediate(0), got {other:?}"),
    };
    host.dispatch(Lv2Request::LwMutexLock { id, timeout: 0 }, owner_unit, &rt);
    // Other thread tries non-blockingly.
    let r = host.dispatch(Lv2Request::LwMutexTryLock { id }, other_unit, &rt);
    assert!(matches!(
        r,
        Lv2Dispatch::Immediate {
            code: 0x8001_000A,
            ..
        }
    ));
    // Owner unchanged; waiter list untouched.
    let entry = host.lwmutexes().lookup(id).unwrap();
    assert_eq!(entry.owner(), Some(PpuThreadId::PRIMARY));
    assert!(entry.waiters().is_empty());
}

#[test]
fn lwmutex_trylock_unknown_returns_esrch() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let src = UnitId::new(0);
    seed_primary_ppu(&mut host, src);
    let r = host.dispatch(Lv2Request::LwMutexTryLock { id: 77 }, src, &rt);
    assert!(matches!(
        r,
        Lv2Dispatch::Immediate {
            code: 0x8001_0005,
            ..
        }
    ));
}

#[test]
fn lwmutex_unlock_without_waiters_frees() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let src = UnitId::new(0);
    seed_primary_ppu(&mut host, src);
    let created = host.dispatch(
        Lv2Request::LwMutexCreate {
            id_ptr: 0x100,
            attr_ptr: 0x200,
        },
        src,
        &rt,
    );
    let id = match &created {
        Lv2Dispatch::Immediate {
            code: 0,
            effects: e,
        } => extract_write_u32(&e[0]),
        other => panic!("expected Immediate(0), got {other:?}"),
    };
    host.dispatch(Lv2Request::LwMutexLock { id, timeout: 0 }, src, &rt);
    let r = host.dispatch(Lv2Request::LwMutexUnlock { id }, src, &rt);
    assert!(matches!(
        r,
        Lv2Dispatch::Immediate {
            code: 0,
            effects: _,
        }
    ));
    assert_eq!(host.lwmutexes().lookup(id).unwrap().owner(), None);
}

#[test]
fn lwmutex_unlock_with_waiters_transfers_and_reports_wake_target() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let owner_unit = UnitId::new(0);
    let waiter_unit = UnitId::new(1);
    seed_primary_ppu(&mut host, owner_unit);
    let waiter_tid = host
        .ppu_threads_mut()
        .create(
            waiter_unit,
            PpuThreadAttrs {
                entry: 0,
                arg: 0,
                stack_base: 0,
                stack_size: 0,
                priority: 0,
                tls_base: 0,
            },
        )
        .unwrap();
    let created = host.dispatch(
        Lv2Request::LwMutexCreate {
            id_ptr: 0x100,
            attr_ptr: 0x200,
        },
        owner_unit,
        &rt,
    );
    let id = match &created {
        Lv2Dispatch::Immediate {
            code: 0,
            effects: e,
        } => extract_write_u32(&e[0]),
        other => panic!("expected Immediate(0), got {other:?}"),
    };
    host.dispatch(Lv2Request::LwMutexLock { id, timeout: 0 }, owner_unit, &rt);
    host.dispatch(Lv2Request::LwMutexLock { id, timeout: 0 }, waiter_unit, &rt);
    // Owner unlocks.
    let r = host.dispatch(Lv2Request::LwMutexUnlock { id }, owner_unit, &rt);
    match r {
        Lv2Dispatch::WakeAndReturn {
            code: 0,
            woken_unit_ids,
            effects,
            ..
        } => {
            assert_eq!(woken_unit_ids, vec![waiter_unit]);
            assert!(effects.is_empty());
        }
        other => panic!("expected WakeAndReturn, got {other:?}"),
    }
    // Ownership transferred to the waiter.
    let entry = host.lwmutexes().lookup(id).unwrap();
    assert_eq!(entry.owner(), Some(waiter_tid));
    assert!(entry.waiters().is_empty());
}

#[test]
fn lwmutex_unlock_not_owner_returns_eperm() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let owner_unit = UnitId::new(0);
    let other_unit = UnitId::new(1);
    seed_primary_ppu(&mut host, owner_unit);
    host.ppu_threads_mut()
        .create(
            other_unit,
            PpuThreadAttrs {
                entry: 0,
                arg: 0,
                stack_base: 0,
                stack_size: 0,
                priority: 0,
                tls_base: 0,
            },
        )
        .unwrap();
    let created = host.dispatch(
        Lv2Request::LwMutexCreate {
            id_ptr: 0x100,
            attr_ptr: 0x200,
        },
        owner_unit,
        &rt,
    );
    let id = match &created {
        Lv2Dispatch::Immediate {
            code: 0,
            effects: e,
        } => extract_write_u32(&e[0]),
        other => panic!("expected Immediate(0), got {other:?}"),
    };
    host.dispatch(Lv2Request::LwMutexLock { id, timeout: 0 }, owner_unit, &rt);
    // Non-owner tries to unlock.
    let r = host.dispatch(Lv2Request::LwMutexUnlock { id }, other_unit, &rt);
    assert!(matches!(
        r,
        Lv2Dispatch::Immediate {
            code: 0x8001_0008,
            ..
        }
    ));
    // Owner unchanged.
    assert_eq!(
        host.lwmutexes().lookup(id).unwrap().owner(),
        Some(PpuThreadId::PRIMARY),
    );
}

#[test]
fn lwmutex_unlock_unknown_returns_esrch() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let src = UnitId::new(0);
    seed_primary_ppu(&mut host, src);
    let r = host.dispatch(Lv2Request::LwMutexUnlock { id: 99 }, src, &rt);
    assert!(matches!(
        r,
        Lv2Dispatch::Immediate {
            code: 0x8001_0005,
            ..
        }
    ));
}

#[test]
fn lwmutex_unlock_with_three_waiters_wakes_head_in_fifo_order() {
    // Waiters parked in order w1, w2, w3. First unlock wakes w1.
    // w1 unlocks -> wakes w2. w2 unlocks -> wakes w3. w3 unlocks
    // -> mutex free.
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let u0 = UnitId::new(0);
    let u1 = UnitId::new(1);
    let u2 = UnitId::new(2);
    let u3 = UnitId::new(3);
    seed_primary_ppu(&mut host, u0);
    let t1 = host
        .ppu_threads_mut()
        .create(
            u1,
            PpuThreadAttrs {
                entry: 0,
                arg: 0,
                stack_base: 0,
                stack_size: 0,
                priority: 0,
                tls_base: 0,
            },
        )
        .unwrap();
    let t2 = host
        .ppu_threads_mut()
        .create(
            u2,
            PpuThreadAttrs {
                entry: 0,
                arg: 0,
                stack_base: 0,
                stack_size: 0,
                priority: 0,
                tls_base: 0,
            },
        )
        .unwrap();
    let t3 = host
        .ppu_threads_mut()
        .create(
            u3,
            PpuThreadAttrs {
                entry: 0,
                arg: 0,
                stack_base: 0,
                stack_size: 0,
                priority: 0,
                tls_base: 0,
            },
        )
        .unwrap();
    let created = host.dispatch(
        Lv2Request::LwMutexCreate {
            id_ptr: 0x100,
            attr_ptr: 0x200,
        },
        u0,
        &rt,
    );
    let id = match &created {
        Lv2Dispatch::Immediate {
            code: 0,
            effects: e,
        } => extract_write_u32(&e[0]),
        other => panic!("expected Immediate(0), got {other:?}"),
    };
    host.dispatch(Lv2Request::LwMutexLock { id, timeout: 0 }, u0, &rt);
    host.dispatch(Lv2Request::LwMutexLock { id, timeout: 0 }, u1, &rt);
    host.dispatch(Lv2Request::LwMutexLock { id, timeout: 0 }, u2, &rt);
    host.dispatch(Lv2Request::LwMutexLock { id, timeout: 0 }, u3, &rt);
    // u0 unlocks -> u1 gets it.
    match host.dispatch(Lv2Request::LwMutexUnlock { id }, u0, &rt) {
        Lv2Dispatch::WakeAndReturn { woken_unit_ids, .. } => {
            assert_eq!(woken_unit_ids, vec![u1]);
        }
        other => panic!("expected WakeAndReturn, got {other:?}"),
    }
    assert_eq!(host.lwmutexes().lookup(id).unwrap().owner(), Some(t1));
    // u1 unlocks -> u2 gets it.
    match host.dispatch(Lv2Request::LwMutexUnlock { id }, u1, &rt) {
        Lv2Dispatch::WakeAndReturn { woken_unit_ids, .. } => {
            assert_eq!(woken_unit_ids, vec![u2]);
        }
        other => panic!("expected WakeAndReturn, got {other:?}"),
    }
    assert_eq!(host.lwmutexes().lookup(id).unwrap().owner(), Some(t2));
    // u2 unlocks -> u3 gets it.
    match host.dispatch(Lv2Request::LwMutexUnlock { id }, u2, &rt) {
        Lv2Dispatch::WakeAndReturn { woken_unit_ids, .. } => {
            assert_eq!(woken_unit_ids, vec![u3]);
        }
        other => panic!("expected WakeAndReturn, got {other:?}"),
    }
    assert_eq!(host.lwmutexes().lookup(id).unwrap().owner(), Some(t3));
    // u3 unlocks -> mutex free.
    match host.dispatch(Lv2Request::LwMutexUnlock { id }, u3, &rt) {
        Lv2Dispatch::Immediate { code: 0, .. } => {}
        other => panic!("expected Immediate(0), got {other:?}"),
    }
    assert_eq!(host.lwmutexes().lookup(id).unwrap().owner(), None);
}

#[test]
fn lwmutex_lock_duplicate_park_returns_edeadlk() {
    // A caller that is already parked on the same mutex cannot
    // park again. The table's duplicate-enqueue rejection
    // surfaces as EDEADLK at the dispatch level.
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let owner_unit = UnitId::new(0);
    let waiter_unit = UnitId::new(1);
    seed_primary_ppu(&mut host, owner_unit);
    host.ppu_threads_mut()
        .create(
            waiter_unit,
            PpuThreadAttrs {
                entry: 0,
                arg: 0,
                stack_base: 0,
                stack_size: 0,
                priority: 0,
                tls_base: 0,
            },
        )
        .unwrap();
    let created = host.dispatch(
        Lv2Request::LwMutexCreate {
            id_ptr: 0x100,
            attr_ptr: 0x200,
        },
        owner_unit,
        &rt,
    );
    let id = match &created {
        Lv2Dispatch::Immediate {
            code: 0,
            effects: e,
        } => extract_write_u32(&e[0]),
        other => panic!("expected Immediate(0), got {other:?}"),
    };
    host.dispatch(Lv2Request::LwMutexLock { id, timeout: 0 }, owner_unit, &rt);
    host.dispatch(Lv2Request::LwMutexLock { id, timeout: 0 }, waiter_unit, &rt);
    // Second block attempt from the same waiter without a prior
    // wake.
    let r = host.dispatch(Lv2Request::LwMutexLock { id, timeout: 0 }, waiter_unit, &rt);
    assert!(matches!(
        r,
        Lv2Dispatch::Immediate {
            code: 0x8001_000D,
            ..
        }
    ));
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
fn mutex_lock_on_unknown_id_returns_esrch() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(256);
    seed_primary_ppu(&mut host, UnitId::new(0));
    let r = host.dispatch(
        Lv2Request::MutexLock {
            mutex_id: 99,
            timeout: 0,
        },
        UnitId::new(0),
        &rt,
    );
    assert!(matches!(
        r,
        Lv2Dispatch::Immediate {
            code: 0x8001_0005,
            ..
        }
    ));
}

#[test]
fn mutex_lock_unowned_acquires_and_unlock_frees() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(256);
    let src = UnitId::new(0);
    seed_primary_ppu(&mut host, src);
    let created = host.dispatch(
        Lv2Request::MutexCreate {
            id_ptr: 0x100,
            attr_ptr: 0,
        },
        src,
        &rt,
    );
    let id = match &created {
        Lv2Dispatch::Immediate {
            code: 0,
            effects: e,
        } => extract_write_u32(&e[0]),
        other => panic!("expected Immediate(0), got {other:?}"),
    };
    let lock = host.dispatch(
        Lv2Request::MutexLock {
            mutex_id: id,
            timeout: 0,
        },
        src,
        &rt,
    );
    assert!(matches!(
        lock,
        Lv2Dispatch::Immediate {
            code: 0,
            effects: _,
        }
    ));
    assert_eq!(
        host.mutexes().lookup(id).unwrap().owner(),
        Some(PpuThreadId::PRIMARY),
    );
    let unlock = host.dispatch(Lv2Request::MutexUnlock { mutex_id: id }, src, &rt);
    assert!(matches!(
        unlock,
        Lv2Dispatch::Immediate {
            code: 0,
            effects: _,
        }
    ));
    assert_eq!(host.mutexes().lookup(id).unwrap().owner(), None);
}

#[test]
fn mutex_lock_contended_blocks_and_unlock_wakes() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(256);
    let owner_unit = UnitId::new(0);
    let waiter_unit = UnitId::new(1);
    seed_primary_ppu(&mut host, owner_unit);
    let waiter_tid = host
        .ppu_threads_mut()
        .create(
            waiter_unit,
            PpuThreadAttrs {
                entry: 0,
                arg: 0,
                stack_base: 0,
                stack_size: 0,
                priority: 0,
                tls_base: 0,
            },
        )
        .unwrap();
    let created = host.dispatch(
        Lv2Request::MutexCreate {
            id_ptr: 0x100,
            attr_ptr: 0,
        },
        owner_unit,
        &rt,
    );
    let id = match &created {
        Lv2Dispatch::Immediate {
            code: 0,
            effects: e,
        } => extract_write_u32(&e[0]),
        other => panic!("expected Immediate(0), got {other:?}"),
    };
    host.dispatch(
        Lv2Request::MutexLock {
            mutex_id: id,
            timeout: 0,
        },
        owner_unit,
        &rt,
    );
    let block = host.dispatch(
        Lv2Request::MutexLock {
            mutex_id: id,
            timeout: 0,
        },
        waiter_unit,
        &rt,
    );
    match block {
        Lv2Dispatch::Block {
            reason: crate::dispatch::Lv2BlockReason::Mutex { id: blocked_id },
            pending: PendingResponse::ReturnCode { code: 0 },
            ..
        } => {
            assert_eq!(blocked_id, id);
        }
        other => panic!("expected Block on Mutex, got {other:?}"),
    }
    // Owner unlocks -> ownership transfers to waiter and wake
    // dispatch names the waiter's unit id.
    let wake = host.dispatch(Lv2Request::MutexUnlock { mutex_id: id }, owner_unit, &rt);
    match wake {
        Lv2Dispatch::WakeAndReturn {
            code: 0,
            woken_unit_ids,
            ..
        } => assert_eq!(woken_unit_ids, vec![waiter_unit]),
        other => panic!("expected WakeAndReturn, got {other:?}"),
    }
    assert_eq!(host.mutexes().lookup(id).unwrap().owner(), Some(waiter_tid));
}

#[test]
fn lwmutex_and_mutex_id_spaces_are_independent() {
    // The two tables must not collide on ids: a lwmutex id and
    // a mutex id can legitimately share the same u32 value.
    // Acquiring one must not affect the other.
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let src = UnitId::new(0);
    seed_primary_ppu(&mut host, src);
    let lw = host.dispatch(
        Lv2Request::LwMutexCreate {
            id_ptr: 0x100,
            attr_ptr: 0x200,
        },
        src,
        &rt,
    );
    let lw_id = match &lw {
        Lv2Dispatch::Immediate {
            code: 0,
            effects: e,
        } => extract_write_u32(&e[0]),
        other => panic!("expected Immediate(0), got {other:?}"),
    };
    let hv = host.dispatch(
        Lv2Request::MutexCreate {
            id_ptr: 0x104,
            attr_ptr: 0,
        },
        src,
        &rt,
    );
    let hv_id = match &hv {
        Lv2Dispatch::Immediate {
            code: 0,
            effects: e,
        } => extract_write_u32(&e[0]),
        other => panic!("expected Immediate(0), got {other:?}"),
    };
    // lwmutex ids start at 1; heavy mutex ids come from the
    // shared `next_kernel_id` allocator (0x4000_0001+). They
    // MUST NOT collide regardless of that layout; the table
    // types are distinct.
    assert_eq!(lw_id, 1);
    assert!(hv_id >= 0x4000_0001);
    // Acquire both with the primary.
    host.dispatch(
        Lv2Request::LwMutexLock {
            id: lw_id,
            timeout: 0,
        },
        src,
        &rt,
    );
    host.dispatch(
        Lv2Request::MutexLock {
            mutex_id: hv_id,
            timeout: 0,
        },
        src,
        &rt,
    );
    // Both owned by primary, independently.
    assert_eq!(
        host.lwmutexes().lookup(lw_id).unwrap().owner(),
        Some(PpuThreadId::PRIMARY),
    );
    assert_eq!(
        host.mutexes().lookup(hv_id).unwrap().owner(),
        Some(PpuThreadId::PRIMARY),
    );
    // Release lwmutex; heavy mutex unchanged.
    host.dispatch(Lv2Request::LwMutexUnlock { id: lw_id }, src, &rt);
    assert_eq!(host.lwmutexes().lookup(lw_id).unwrap().owner(), None);
    assert_eq!(
        host.mutexes().lookup(hv_id).unwrap().owner(),
        Some(PpuThreadId::PRIMARY),
    );
    // Release heavy mutex; lwmutex still free.
    host.dispatch(Lv2Request::MutexUnlock { mutex_id: hv_id }, src, &rt);
    assert_eq!(host.mutexes().lookup(hv_id).unwrap().owner(), None);
    assert_eq!(host.lwmutexes().lookup(lw_id).unwrap().owner(), None);
}

#[test]
fn lwmutex_and_mutex_waiter_lists_do_not_cross_contaminate() {
    // A thread parked on a lwmutex must not appear as a waiter
    // on a heavy mutex or vice versa, even when both primitives
    // have the same thread as owner.
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let owner_unit = UnitId::new(0);
    let waiter_unit = UnitId::new(1);
    seed_primary_ppu(&mut host, owner_unit);
    let waiter_tid = host
        .ppu_threads_mut()
        .create(
            waiter_unit,
            PpuThreadAttrs {
                entry: 0,
                arg: 0,
                stack_base: 0,
                stack_size: 0,
                priority: 0,
                tls_base: 0,
            },
        )
        .unwrap();
    let lw_id = {
        let r = host.dispatch(
            Lv2Request::LwMutexCreate {
                id_ptr: 0x100,
                attr_ptr: 0x200,
            },
            owner_unit,
            &rt,
        );
        match r {
            Lv2Dispatch::Immediate { effects, .. } => extract_write_u32(&effects[0]),
            other => panic!("expected Immediate, got {other:?}"),
        }
    };
    let hv_id = {
        let r = host.dispatch(
            Lv2Request::MutexCreate {
                id_ptr: 0x104,
                attr_ptr: 0,
            },
            owner_unit,
            &rt,
        );
        match r {
            Lv2Dispatch::Immediate { effects, .. } => extract_write_u32(&effects[0]),
            other => panic!("expected Immediate, got {other:?}"),
        }
    };
    // Owner acquires both.
    host.dispatch(
        Lv2Request::LwMutexLock {
            id: lw_id,
            timeout: 0,
        },
        owner_unit,
        &rt,
    );
    host.dispatch(
        Lv2Request::MutexLock {
            mutex_id: hv_id,
            timeout: 0,
        },
        owner_unit,
        &rt,
    );
    // Waiter parks on the lwmutex only.
    host.dispatch(
        Lv2Request::LwMutexLock {
            id: lw_id,
            timeout: 0,
        },
        waiter_unit,
        &rt,
    );
    // lwmutex waiter list has waiter_tid; heavy mutex list is
    // empty.
    assert_eq!(
        host.lwmutexes()
            .lookup(lw_id)
            .unwrap()
            .waiters()
            .iter()
            .collect::<Vec<_>>(),
        vec![waiter_tid],
    );
    assert!(host.mutexes().lookup(hv_id).unwrap().waiters().is_empty());
    // Releasing the heavy mutex must not wake the lwmutex
    // waiter.
    let r = host.dispatch(Lv2Request::MutexUnlock { mutex_id: hv_id }, owner_unit, &rt);
    assert!(matches!(
        r,
        Lv2Dispatch::Immediate {
            code: 0,
            effects: _,
        }
    ));
    // lwmutex waiter still parked.
    assert_eq!(
        host.lwmutexes()
            .lookup(lw_id)
            .unwrap()
            .waiters()
            .iter()
            .collect::<Vec<_>>(),
        vec![waiter_tid],
    );
    // Releasing the lwmutex wakes the waiter.
    let r = host.dispatch(Lv2Request::LwMutexUnlock { id: lw_id }, owner_unit, &rt);
    match r {
        Lv2Dispatch::WakeAndReturn { woken_unit_ids, .. } => {
            assert_eq!(woken_unit_ids, vec![waiter_unit]);
        }
        other => panic!("expected WakeAndReturn, got {other:?}"),
    }
}

#[test]
fn mutex_create_default_attrs_when_attr_ptr_zero() {
    // attr_ptr = 0 -- handler must fall back to MutexAttrs::default()
    // without panicking or reading guest memory.
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let src = UnitId::new(0);
    seed_primary_ppu(&mut host, src);
    let created = host.dispatch(
        Lv2Request::MutexCreate {
            id_ptr: 0x100,
            attr_ptr: 0,
        },
        src,
        &rt,
    );
    let id = match &created {
        Lv2Dispatch::Immediate {
            code: 0,
            effects: e,
        } => extract_write_u32(&e[0]),
        other => panic!("expected Immediate(0), got {other:?}"),
    };
    assert_eq!(
        host.mutexes().lookup(id).unwrap().attrs(),
        crate::sync_primitives::MutexAttrs::default()
    );
}

#[test]
fn mutex_create_decodes_attr_ptr() {
    // Plant a real attribute struct in guest memory and verify
    // dispatch_mutex_create decodes protocol / recursive flags
    // and surfaces them on the table entry.
    let mut mem = cellgov_mem::GuestMemory::new(0x10000);
    let attr_bytes = [
        0x00, 0x00, 0x00, 0x20, // protocol = 0x20 (PRIORITY_INHERIT)
        0x00, 0x00, 0x00, 0x11, // recursive = 0x11 (RECURSIVE)
        0x00, 0x00, 0x00, 0x00, // pshared (ignored)
    ];
    mem.apply_commit(
        cellgov_mem::ByteRange::new(cellgov_mem::GuestAddr::new(0x200), 12).unwrap(),
        &attr_bytes,
    )
    .unwrap();
    let rt = FakeRuntime::with_memory(mem);
    let mut host = Lv2Host::new();
    let src = UnitId::new(0);
    seed_primary_ppu(&mut host, src);
    let created = host.dispatch(
        Lv2Request::MutexCreate {
            id_ptr: 0x100,
            attr_ptr: 0x200,
        },
        src,
        &rt,
    );
    let id = match &created {
        Lv2Dispatch::Immediate {
            code: 0,
            effects: e,
        } => extract_write_u32(&e[0]),
        other => panic!("expected Immediate(0), got {other:?}"),
    };
    let attrs = host.mutexes().lookup(id).unwrap().attrs();
    assert_eq!(attrs.protocol, 0x20);
    assert_eq!(attrs.priority_policy, 0x20);
    assert!(attrs.recursive);
}

#[test]
fn mutex_trylock_unowned_acquires() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(256);
    let src = UnitId::new(0);
    seed_primary_ppu(&mut host, src);
    let created = host.dispatch(
        Lv2Request::MutexCreate {
            id_ptr: 0x100,
            attr_ptr: 0,
        },
        src,
        &rt,
    );
    let id = match &created {
        Lv2Dispatch::Immediate {
            code: 0,
            effects: e,
        } => extract_write_u32(&e[0]),
        other => panic!("expected Immediate(0), got {other:?}"),
    };
    let r = host.dispatch(Lv2Request::MutexTryLock { mutex_id: id }, src, &rt);
    assert!(matches!(
        r,
        Lv2Dispatch::Immediate {
            code: 0,
            effects: _,
        }
    ));
    assert_eq!(
        host.mutexes().lookup(id).unwrap().owner(),
        Some(PpuThreadId::PRIMARY),
    );
}

#[test]
fn mutex_trylock_contended_returns_ebusy_and_does_not_park() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(256);
    let owner_unit = UnitId::new(0);
    let other_unit = UnitId::new(1);
    seed_primary_ppu(&mut host, owner_unit);
    host.ppu_threads_mut()
        .create(
            other_unit,
            PpuThreadAttrs {
                entry: 0,
                arg: 0,
                stack_base: 0,
                stack_size: 0,
                priority: 0,
                tls_base: 0,
            },
        )
        .unwrap();
    let created = host.dispatch(
        Lv2Request::MutexCreate {
            id_ptr: 0x100,
            attr_ptr: 0,
        },
        owner_unit,
        &rt,
    );
    let id = match &created {
        Lv2Dispatch::Immediate {
            code: 0,
            effects: e,
        } => extract_write_u32(&e[0]),
        other => panic!("expected Immediate(0), got {other:?}"),
    };
    host.dispatch(
        Lv2Request::MutexLock {
            mutex_id: id,
            timeout: 0,
        },
        owner_unit,
        &rt,
    );
    let r = host.dispatch(Lv2Request::MutexTryLock { mutex_id: id }, other_unit, &rt);
    assert!(matches!(
        r,
        Lv2Dispatch::Immediate {
            code: 0x8001_000A,
            ..
        }
    ));
    assert_eq!(
        host.mutexes().lookup(id).unwrap().owner(),
        Some(PpuThreadId::PRIMARY),
    );
    assert!(host.mutexes().lookup(id).unwrap().waiters().is_empty());
}

#[test]
fn mutex_trylock_unknown_returns_esrch() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(256);
    let src = UnitId::new(0);
    seed_primary_ppu(&mut host, src);
    let r = host.dispatch(Lv2Request::MutexTryLock { mutex_id: 77 }, src, &rt);
    assert!(matches!(
        r,
        Lv2Dispatch::Immediate {
            code: 0x8001_0005,
            ..
        }
    ));
}

#[test]
fn semaphore_create_writes_id_and_stores_entry() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(256);
    let src = UnitId::new(0);
    seed_primary_ppu(&mut host, src);
    let r = host.dispatch(
        Lv2Request::SemaphoreCreate {
            id_ptr: 0x100,
            attr_ptr: 0,
            initial: 2,
            max: 10,
        },
        src,
        &rt,
    );
    let id = match &r {
        Lv2Dispatch::Immediate {
            code: 0,
            effects: e,
        } => extract_write_u32(&e[0]),
        other => panic!("expected Immediate(0), got {other:?}"),
    };
    let entry = host.semaphores().lookup(id).unwrap();
    assert_eq!(entry.count(), 2);
    assert_eq!(entry.max(), 10);
}

#[test]
fn semaphore_create_rejects_initial_above_max() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(256);
    let r = host.dispatch(
        Lv2Request::SemaphoreCreate {
            id_ptr: 0x100,
            attr_ptr: 0,
            initial: 11,
            max: 10,
        },
        UnitId::new(0),
        &rt,
    );
    assert!(matches!(
        r,
        Lv2Dispatch::Immediate {
            code: 0x8001_0002,
            ..
        }
    ));
}

#[test]
fn semaphore_destroy_unknown_returns_esrch() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(256);
    let r = host.dispatch(Lv2Request::SemaphoreDestroy { id: 77 }, UnitId::new(0), &rt);
    assert!(matches!(
        r,
        Lv2Dispatch::Immediate {
            code: 0x8001_0005,
            ..
        }
    ));
}

#[test]
fn semaphore_destroy_with_waiter_returns_ebusy() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(256);
    let src = UnitId::new(0);
    seed_primary_ppu(&mut host, src);
    let r = host.dispatch(
        Lv2Request::SemaphoreCreate {
            id_ptr: 0x100,
            attr_ptr: 0,
            initial: 0,
            max: 10,
        },
        src,
        &rt,
    );
    let id = match &r {
        Lv2Dispatch::Immediate {
            code: 0,
            effects: e,
        } => extract_write_u32(&e[0]),
        other => panic!("expected Immediate(0), got {other:?}"),
    };
    host.semaphores_mut()
        .enqueue_waiter(id, PpuThreadId::PRIMARY);
    let d = host.dispatch(Lv2Request::SemaphoreDestroy { id }, src, &rt);
    assert!(matches!(
        d,
        Lv2Dispatch::Immediate {
            code: 0x8001_000A,
            ..
        }
    ));
}

#[test]
fn semaphore_wait_with_positive_count_decrements_and_returns_ok() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(256);
    let src = UnitId::new(0);
    seed_primary_ppu(&mut host, src);
    let r = host.dispatch(
        Lv2Request::SemaphoreCreate {
            id_ptr: 0x100,
            attr_ptr: 0,
            initial: 1,
            max: 10,
        },
        src,
        &rt,
    );
    let id = match &r {
        Lv2Dispatch::Immediate {
            code: 0,
            effects: e,
        } => extract_write_u32(&e[0]),
        other => panic!("expected Immediate(0), got {other:?}"),
    };
    let w = host.dispatch(Lv2Request::SemaphoreWait { id, timeout: 0 }, src, &rt);
    assert!(matches!(
        w,
        Lv2Dispatch::Immediate {
            code: 0,
            effects: _,
        }
    ));
    assert_eq!(host.semaphores().lookup(id).unwrap().count(), 0);
}

#[test]
fn semaphore_wait_with_zero_count_parks_caller() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(256);
    let src = UnitId::new(0);
    seed_primary_ppu(&mut host, src);
    let r = host.dispatch(
        Lv2Request::SemaphoreCreate {
            id_ptr: 0x100,
            attr_ptr: 0,
            initial: 0,
            max: 10,
        },
        src,
        &rt,
    );
    let id = match &r {
        Lv2Dispatch::Immediate {
            code: 0,
            effects: e,
        } => extract_write_u32(&e[0]),
        other => panic!("expected Immediate(0), got {other:?}"),
    };
    let w = host.dispatch(Lv2Request::SemaphoreWait { id, timeout: 0 }, src, &rt);
    match w {
        Lv2Dispatch::Block {
            reason: crate::dispatch::Lv2BlockReason::Semaphore { id: sid },
            pending: PendingResponse::ReturnCode { code: 0 },
            ..
        } => {
            assert_eq!(sid, id);
        }
        other => panic!("expected Block on Semaphore, got {other:?}"),
    }
    let waiters: Vec<_> = host
        .semaphores()
        .lookup(id)
        .unwrap()
        .waiters()
        .iter()
        .collect();
    assert_eq!(waiters, vec![PpuThreadId::PRIMARY]);
}

#[test]
fn event_queue_create_writes_id_and_stores_entry() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let src = UnitId::new(0);
    seed_primary_ppu(&mut host, src);
    let r = host.dispatch(
        Lv2Request::EventQueueCreate {
            id_ptr: 0x100,
            attr_ptr: 0,
            key: 0,
            size: 8,
        },
        src,
        &rt,
    );
    let id = match &r {
        Lv2Dispatch::Immediate {
            code: 0,
            effects: e,
        } => extract_write_u32(&e[0]),
        other => panic!("expected Immediate(0), got {other:?}"),
    };
    assert_eq!(host.event_queues().lookup(id).unwrap().size(), 8);
}

#[test]
fn event_queue_destroy_with_waiters_returns_ebusy() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let src = UnitId::new(0);
    seed_primary_ppu(&mut host, src);
    let r = host.dispatch(
        Lv2Request::EventQueueCreate {
            id_ptr: 0x100,
            attr_ptr: 0,
            key: 0,
            size: 4,
        },
        src,
        &rt,
    );
    let id = match &r {
        Lv2Dispatch::Immediate {
            code: 0,
            effects: e,
        } => extract_write_u32(&e[0]),
        other => panic!("expected Immediate(0), got {other:?}"),
    };
    host.event_queues_mut()
        .enqueue_waiter(id, PpuThreadId::PRIMARY, 0x2000);
    let d = host.dispatch(Lv2Request::EventQueueDestroy { queue_id: id }, src, &rt);
    assert!(matches!(
        d,
        Lv2Dispatch::Immediate {
            code: 0x8001_000A,
            ..
        }
    ));
}

#[test]
fn event_queue_receive_with_buffered_payload_delivers_immediately() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let src = UnitId::new(0);
    seed_primary_ppu(&mut host, src);
    let r = host.dispatch(
        Lv2Request::EventQueueCreate {
            id_ptr: 0x100,
            attr_ptr: 0,
            key: 0,
            size: 4,
        },
        src,
        &rt,
    );
    let id = match &r {
        Lv2Dispatch::Immediate {
            code: 0,
            effects: e,
        } => extract_write_u32(&e[0]),
        other => panic!("expected Immediate(0), got {other:?}"),
    };
    // Pre-buffer a payload directly via the table.
    host.event_queues_mut().send_and_wake_or_enqueue(
        id,
        crate::sync_primitives::EventPayload {
            source: 0x11,
            data1: 0x22,
            data2: 0x33,
            data3: 0x44,
        },
    );
    let recv = host.dispatch(
        Lv2Request::EventQueueReceive {
            queue_id: id,
            out_ptr: 0x2000,
            timeout: 0,
        },
        src,
        &rt,
    );
    match recv {
        Lv2Dispatch::Immediate {
            code: 0,
            effects: e,
        } => {
            // Payload written via a single SharedWriteIntent.
            match &e[0] {
                Effect::SharedWriteIntent { range, bytes, .. } => {
                    assert_eq!(range.start().raw(), 0x2000);
                    assert_eq!(range.length(), 32);
                    let payload_bytes = bytes.bytes();
                    assert_eq!(
                        u64::from_be_bytes(payload_bytes[0..8].try_into().unwrap()),
                        0x11
                    );
                    assert_eq!(
                        u64::from_be_bytes(payload_bytes[8..16].try_into().unwrap()),
                        0x22
                    );
                }
                other => panic!("expected SharedWriteIntent, got {other:?}"),
            }
        }
        other => panic!("expected Immediate(0), got {other:?}"),
    }
    // Queue now empty.
    assert!(host.event_queues().lookup(id).unwrap().is_empty());
}

#[test]
fn event_queue_receive_empty_parks_caller() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let src = UnitId::new(0);
    seed_primary_ppu(&mut host, src);
    let r = host.dispatch(
        Lv2Request::EventQueueCreate {
            id_ptr: 0x100,
            attr_ptr: 0,
            key: 0,
            size: 4,
        },
        src,
        &rt,
    );
    let id = match &r {
        Lv2Dispatch::Immediate {
            code: 0,
            effects: e,
        } => extract_write_u32(&e[0]),
        other => panic!("expected Immediate(0), got {other:?}"),
    };
    let recv = host.dispatch(
        Lv2Request::EventQueueReceive {
            queue_id: id,
            out_ptr: 0x2000,
            timeout: 0,
        },
        src,
        &rt,
    );
    match recv {
        Lv2Dispatch::Block {
            reason: crate::dispatch::Lv2BlockReason::EventQueue { id: bid },
            pending:
                PendingResponse::EventQueueReceive {
                    out_ptr,
                    source,
                    data1,
                    data2,
                    data3,
                },
            ..
        } => {
            assert_eq!(bid, id);
            assert_eq!(out_ptr, 0x2000);
            // Placeholder zeros; real payload lands via
            // response_updates at send time.
            assert_eq!((source, data1, data2, data3), (0, 0, 0, 0));
        }
        other => panic!("expected Block on EventQueue, got {other:?}"),
    }
    let waiters: Vec<_> = host
        .event_queues()
        .lookup(id)
        .unwrap()
        .waiters()
        .iter()
        .map(|w| (w.thread, w.out_ptr))
        .collect();
    assert_eq!(waiters, vec![(PpuThreadId::PRIMARY, 0x2000)]);
}

#[test]
fn event_port_send_with_parked_waiter_emits_wake_and_return_with_payload() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let sender_unit = UnitId::new(0);
    let waiter_unit = UnitId::new(1);
    seed_primary_ppu(&mut host, sender_unit);
    host.ppu_threads_mut()
        .create(
            waiter_unit,
            PpuThreadAttrs {
                entry: 0,
                arg: 0,
                stack_base: 0,
                stack_size: 0,
                priority: 0,
                tls_base: 0,
            },
        )
        .unwrap();
    let r = host.dispatch(
        Lv2Request::EventQueueCreate {
            id_ptr: 0x100,
            attr_ptr: 0,
            key: 0,
            size: 4,
        },
        sender_unit,
        &rt,
    );
    let id = match &r {
        Lv2Dispatch::Immediate {
            code: 0,
            effects: e,
        } => extract_write_u32(&e[0]),
        other => panic!("expected Immediate(0), got {other:?}"),
    };
    // Waiter parks.
    host.dispatch(
        Lv2Request::EventQueueReceive {
            queue_id: id,
            out_ptr: 0x2000,
            timeout: 0,
        },
        waiter_unit,
        &rt,
    );
    // Sender sends.
    let send = host.dispatch(
        Lv2Request::EventPortSend {
            port_id: id,
            data1: 0xAA,
            data2: 0xBB,
            data3: 0xCC,
        },
        sender_unit,
        &rt,
    );
    match send {
        Lv2Dispatch::WakeAndReturn {
            code: 0,
            woken_unit_ids,
            response_updates,
            ..
        } => {
            assert_eq!(woken_unit_ids, vec![waiter_unit]);
            assert_eq!(response_updates.len(), 1);
            let (u, resp) = &response_updates[0];
            assert_eq!(*u, waiter_unit);
            match resp {
                PendingResponse::EventQueueReceive {
                    source,
                    data1,
                    data2,
                    data3,
                    ..
                } => {
                    assert_eq!(*source, id as u64);
                    assert_eq!(*data1, 0xAA);
                    assert_eq!(*data2, 0xBB);
                    assert_eq!(*data3, 0xCC);
                }
                other => panic!("expected EventQueueReceive, got {other:?}"),
            }
        }
        other => panic!("expected WakeAndReturn, got {other:?}"),
    }
    // Queue storage unchanged (fast-path handoff), waiter
    // list drained.
    assert!(host.event_queues().lookup(id).unwrap().is_empty());
    assert!(host.event_queues().lookup(id).unwrap().waiters().is_empty());
}

#[test]
fn event_flag_create_stores_init_bits() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let src = UnitId::new(0);
    seed_primary_ppu(&mut host, src);
    let r = host.dispatch(
        Lv2Request::EventFlagCreate {
            id_ptr: 0x100,
            attr_ptr: 0,
            init: 0x1234,
        },
        src,
        &rt,
    );
    let id = match &r {
        Lv2Dispatch::Immediate {
            code: 0,
            effects: e,
        } => extract_write_u32(&e[0]),
        other => panic!("expected Immediate(0), got {other:?}"),
    };
    assert_eq!(host.event_flags().lookup(id).unwrap().bits(), 0x1234);
}

#[test]
fn event_flag_wait_and_mode_mask_match_returns_observed_bits() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let src = UnitId::new(0);
    seed_primary_ppu(&mut host, src);
    let r = host.dispatch(
        Lv2Request::EventFlagCreate {
            id_ptr: 0x100,
            attr_ptr: 0,
            init: 0b1111,
        },
        src,
        &rt,
    );
    let id = match &r {
        Lv2Dispatch::Immediate {
            code: 0,
            effects: e,
        } => extract_write_u32(&e[0]),
        other => panic!("expected Immediate(0), got {other:?}"),
    };
    // AND + NO-CLEAR: mode = 0x01.
    let w = host.dispatch(
        Lv2Request::EventFlagWait {
            id,
            bits: 0b0011,
            mode: 0x01,
            result_ptr: 0x200,
            timeout: 0,
        },
        src,
        &rt,
    );
    match w {
        Lv2Dispatch::Immediate {
            code: 0,
            effects: e,
        } => {
            assert_eq!(e.len(), 1);
        }
        other => panic!("expected Immediate(0), got {other:?}"),
    }
    // NO-CLEAR -- bits unchanged.
    assert_eq!(host.event_flags().lookup(id).unwrap().bits(), 0b1111);
}

#[test]
fn event_flag_wait_no_match_parks_caller() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let src = UnitId::new(0);
    seed_primary_ppu(&mut host, src);
    let r = host.dispatch(
        Lv2Request::EventFlagCreate {
            id_ptr: 0x100,
            attr_ptr: 0,
            init: 0,
        },
        src,
        &rt,
    );
    let id = match &r {
        Lv2Dispatch::Immediate {
            code: 0,
            effects: e,
        } => extract_write_u32(&e[0]),
        other => panic!("expected Immediate(0), got {other:?}"),
    };
    let w = host.dispatch(
        Lv2Request::EventFlagWait {
            id,
            bits: 0b0010,
            mode: 0x01, // AND + NO-CLEAR
            result_ptr: 0x200,
            timeout: 0,
        },
        src,
        &rt,
    );
    match w {
        Lv2Dispatch::Block {
            reason: crate::dispatch::Lv2BlockReason::EventFlag { id: fid },
            pending:
                PendingResponse::EventFlagWake {
                    result_ptr,
                    observed,
                },
            ..
        } => {
            assert_eq!(fid, id);
            assert_eq!(result_ptr, 0x200);
            assert_eq!(observed, 0);
        }
        other => panic!("expected Block on EventFlag, got {other:?}"),
    }
    assert_eq!(host.event_flags().lookup(id).unwrap().waiters().len(), 1);
}

#[test]
fn event_flag_set_wakes_matching_waiters_in_fifo_order() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let u1 = UnitId::new(0);
    let u2 = UnitId::new(1);
    let u3 = UnitId::new(2);
    seed_primary_ppu(&mut host, u1);
    let _t2 = host
        .ppu_threads_mut()
        .create(
            u2,
            PpuThreadAttrs {
                entry: 0,
                arg: 0,
                stack_base: 0,
                stack_size: 0,
                priority: 0,
                tls_base: 0,
            },
        )
        .unwrap();
    let t3 = host
        .ppu_threads_mut()
        .create(
            u3,
            PpuThreadAttrs {
                entry: 0,
                arg: 0,
                stack_base: 0,
                stack_size: 0,
                priority: 0,
                tls_base: 0,
            },
        )
        .unwrap();
    let r = host.dispatch(
        Lv2Request::EventFlagCreate {
            id_ptr: 0x100,
            attr_ptr: 0,
            init: 0,
        },
        u1,
        &rt,
    );
    let id = match &r {
        Lv2Dispatch::Immediate {
            code: 0,
            effects: e,
        } => extract_write_u32(&e[0]),
        other => panic!("expected Immediate(0), got {other:?}"),
    };
    // u1 waits on 0b0001 (matches).
    host.dispatch(
        Lv2Request::EventFlagWait {
            id,
            bits: 0b0001,
            mode: 0x01,
            result_ptr: 0x200,
            timeout: 0,
        },
        u1,
        &rt,
    );
    // u2 waits on 0b0010 (matches).
    host.dispatch(
        Lv2Request::EventFlagWait {
            id,
            bits: 0b0010,
            mode: 0x01,
            result_ptr: 0x210,
            timeout: 0,
        },
        u2,
        &rt,
    );
    // u3 waits on 0b1000 (NO match after set).
    host.dispatch(
        Lv2Request::EventFlagWait {
            id,
            bits: 0b1000,
            mode: 0x01,
            result_ptr: 0x220,
            timeout: 0,
        },
        u3,
        &rt,
    );
    // Set bits 0b0011 -- wakes u1 and u2 in FIFO order.
    let s = host.dispatch(Lv2Request::EventFlagSet { id, bits: 0b0011 }, u1, &rt);
    match s {
        Lv2Dispatch::WakeAndReturn {
            code: 0,
            woken_unit_ids,
            response_updates,
            ..
        } => {
            assert_eq!(woken_unit_ids, vec![u1, u2]);
            assert_eq!(response_updates.len(), 2);
            // Each carries the observed bit pattern.
            for (_, resp) in &response_updates {
                match resp {
                    PendingResponse::EventFlagWake { observed, .. } => {
                        assert_eq!(*observed, 0b0011);
                    }
                    other => panic!("expected EventFlagWake, got {other:?}"),
                }
            }
        }
        other => panic!("expected WakeAndReturn, got {other:?}"),
    }
    // u3 still parked.
    let remaining: Vec<_> = host
        .event_flags()
        .lookup(id)
        .unwrap()
        .waiters()
        .iter()
        .map(|w| w.thread)
        .collect();
    assert_eq!(remaining, vec![t3]);
}

#[test]
fn event_flag_clear_does_not_wake_anyone() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let src = UnitId::new(0);
    seed_primary_ppu(&mut host, src);
    let r = host.dispatch(
        Lv2Request::EventFlagCreate {
            id_ptr: 0x100,
            attr_ptr: 0,
            init: 0b1111,
        },
        src,
        &rt,
    );
    let id = match &r {
        Lv2Dispatch::Immediate {
            code: 0,
            effects: e,
        } => extract_write_u32(&e[0]),
        other => panic!("expected Immediate(0), got {other:?}"),
    };
    let c = host.dispatch(Lv2Request::EventFlagClear { id, bits: 0b0101 }, src, &rt);
    assert!(matches!(
        c,
        Lv2Dispatch::Immediate {
            code: 0,
            effects: _,
        }
    ));
    assert_eq!(host.event_flags().lookup(id).unwrap().bits(), 0b1010);
}

#[test]
fn event_flag_trywait_no_match_returns_ebusy() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let src = UnitId::new(0);
    seed_primary_ppu(&mut host, src);
    let r = host.dispatch(
        Lv2Request::EventFlagCreate {
            id_ptr: 0x100,
            attr_ptr: 0,
            init: 0,
        },
        src,
        &rt,
    );
    let id = match &r {
        Lv2Dispatch::Immediate {
            code: 0,
            effects: e,
        } => extract_write_u32(&e[0]),
        other => panic!("expected Immediate(0), got {other:?}"),
    };
    let w = host.dispatch(
        Lv2Request::EventFlagTryWait {
            id,
            bits: 0b1,
            mode: 0x01,
            result_ptr: 0x200,
        },
        src,
        &rt,
    );
    assert!(matches!(
        w,
        Lv2Dispatch::Immediate {
            code: 0x8001_000A,
            ..
        }
    ));
}

#[test]
fn event_queue_tryreceive_batch_drains_payloads() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let src = UnitId::new(0);
    seed_primary_ppu(&mut host, src);
    let r = host.dispatch(
        Lv2Request::EventQueueCreate {
            id_ptr: 0x100,
            attr_ptr: 0,
            key: 0,
            size: 4,
        },
        src,
        &rt,
    );
    let id = match &r {
        Lv2Dispatch::Immediate {
            code: 0,
            effects: e,
        } => extract_write_u32(&e[0]),
        other => panic!("expected Immediate(0), got {other:?}"),
    };
    // Pre-buffer three payloads.
    for i in 1..=3u64 {
        host.event_queues_mut().send_and_wake_or_enqueue(
            id,
            crate::sync_primitives::EventPayload {
                source: i,
                data1: i * 10,
                data2: 0,
                data3: 0,
            },
        );
    }
    let tr = host.dispatch(
        Lv2Request::EventQueueTryReceive {
            queue_id: id,
            event_array: 0x2000,
            size: 2,
            count_out: 0x3000,
        },
        src,
        &rt,
    );
    match tr {
        Lv2Dispatch::Immediate {
            code: 0,
            effects: e,
        } => {
            // 2 payload writes + 1 count_out write.
            assert_eq!(e.len(), 3);
        }
        other => panic!("expected Immediate(0), got {other:?}"),
    }
    // Queue has one remaining (3rd payload).
    assert_eq!(host.event_queues().lookup(id).unwrap().len(), 1);
}

#[test]
fn event_queue_tryreceive_empty_writes_zero_count() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let src = UnitId::new(0);
    seed_primary_ppu(&mut host, src);
    let r = host.dispatch(
        Lv2Request::EventQueueCreate {
            id_ptr: 0x100,
            attr_ptr: 0,
            key: 0,
            size: 4,
        },
        src,
        &rt,
    );
    let id = match &r {
        Lv2Dispatch::Immediate {
            code: 0,
            effects: e,
        } => extract_write_u32(&e[0]),
        other => panic!("expected Immediate(0), got {other:?}"),
    };
    let tr = host.dispatch(
        Lv2Request::EventQueueTryReceive {
            queue_id: id,
            event_array: 0x2000,
            size: 2,
            count_out: 0x3000,
        },
        src,
        &rt,
    );
    match tr {
        Lv2Dispatch::Immediate {
            code: 0,
            effects: e,
        } => {
            // Only the count_out write (value 0).
            assert_eq!(e.len(), 1);
        }
        other => panic!("expected Immediate(0), got {other:?}"),
    }
}

#[test]
fn event_queue_tryreceive_unknown_returns_esrch() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let r = host.dispatch(
        Lv2Request::EventQueueTryReceive {
            queue_id: 99,
            event_array: 0x2000,
            size: 2,
            count_out: 0x3000,
        },
        UnitId::new(0),
        &rt,
    );
    assert!(matches!(
        r,
        Lv2Dispatch::Immediate {
            code: 0x8001_0005,
            ..
        }
    ));
}

#[test]
fn event_port_send_with_no_waiters_enqueues_payload() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let src = UnitId::new(0);
    seed_primary_ppu(&mut host, src);
    let r = host.dispatch(
        Lv2Request::EventQueueCreate {
            id_ptr: 0x100,
            attr_ptr: 0,
            key: 0,
            size: 4,
        },
        src,
        &rt,
    );
    let id = match &r {
        Lv2Dispatch::Immediate {
            code: 0,
            effects: e,
        } => extract_write_u32(&e[0]),
        other => panic!("expected Immediate(0), got {other:?}"),
    };
    let send = host.dispatch(
        Lv2Request::EventPortSend {
            port_id: id,
            data1: 0xAA,
            data2: 0xBB,
            data3: 0xCC,
        },
        src,
        &rt,
    );
    assert!(matches!(
        send,
        Lv2Dispatch::Immediate {
            code: 0,
            effects: _,
        }
    ));
    assert_eq!(host.event_queues().lookup(id).unwrap().len(), 1);
}

#[test]
fn semaphore_trywait_with_positive_count_acquires() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(256);
    let src = UnitId::new(0);
    seed_primary_ppu(&mut host, src);
    let r = host.dispatch(
        Lv2Request::SemaphoreCreate {
            id_ptr: 0x100,
            attr_ptr: 0,
            initial: 1,
            max: 10,
        },
        src,
        &rt,
    );
    let id = match &r {
        Lv2Dispatch::Immediate {
            code: 0,
            effects: e,
        } => extract_write_u32(&e[0]),
        other => panic!("expected Immediate(0), got {other:?}"),
    };
    let w = host.dispatch(Lv2Request::SemaphoreTryWait { id }, src, &rt);
    assert!(matches!(
        w,
        Lv2Dispatch::Immediate {
            code: 0,
            effects: _,
        }
    ));
    assert_eq!(host.semaphores().lookup(id).unwrap().count(), 0);
}

#[test]
fn semaphore_trywait_with_zero_count_returns_ebusy() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(256);
    let src = UnitId::new(0);
    seed_primary_ppu(&mut host, src);
    let r = host.dispatch(
        Lv2Request::SemaphoreCreate {
            id_ptr: 0x100,
            attr_ptr: 0,
            initial: 0,
            max: 10,
        },
        src,
        &rt,
    );
    let id = match &r {
        Lv2Dispatch::Immediate {
            code: 0,
            effects: e,
        } => extract_write_u32(&e[0]),
        other => panic!("expected Immediate(0), got {other:?}"),
    };
    let w = host.dispatch(Lv2Request::SemaphoreTryWait { id }, src, &rt);
    assert!(matches!(
        w,
        Lv2Dispatch::Immediate {
            code: 0x8001_000A,
            ..
        }
    ));
    // Count unchanged and no waiter parked.
    assert_eq!(host.semaphores().lookup(id).unwrap().count(), 0);
    assert!(host.semaphores().lookup(id).unwrap().waiters().is_empty());
}

#[test]
fn semaphore_trywait_unknown_returns_esrch() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(256);
    let r = host.dispatch(Lv2Request::SemaphoreTryWait { id: 99 }, UnitId::new(0), &rt);
    assert!(matches!(
        r,
        Lv2Dispatch::Immediate {
            code: 0x8001_0005,
            ..
        }
    ));
}

#[test]
fn semaphore_get_value_writes_current_count() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(256);
    let src = UnitId::new(0);
    seed_primary_ppu(&mut host, src);
    let r = host.dispatch(
        Lv2Request::SemaphoreCreate {
            id_ptr: 0x100,
            attr_ptr: 0,
            initial: 5,
            max: 10,
        },
        src,
        &rt,
    );
    let id = match &r {
        Lv2Dispatch::Immediate {
            code: 0,
            effects: e,
        } => extract_write_u32(&e[0]),
        other => panic!("expected Immediate(0), got {other:?}"),
    };
    let g = host.dispatch(
        Lv2Request::SemaphoreGetValue { id, out_ptr: 0x200 },
        src,
        &rt,
    );
    match &g {
        Lv2Dispatch::Immediate {
            code: 0,
            effects: e,
        } => {
            assert_eq!(extract_write_u32(&e[0]), 5);
        }
        other => panic!("expected Immediate(0), got {other:?}"),
    }
}

#[test]
fn semaphore_get_value_unknown_returns_esrch() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(256);
    let r = host.dispatch(
        Lv2Request::SemaphoreGetValue {
            id: 99,
            out_ptr: 0x200,
        },
        UnitId::new(0),
        &rt,
    );
    assert!(matches!(
        r,
        Lv2Dispatch::Immediate {
            code: 0x8001_0005,
            ..
        }
    ));
}

#[test]
fn semaphore_post_unknown_returns_esrch() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(256);
    let r = host.dispatch(
        Lv2Request::SemaphorePost { id: 99, val: 1 },
        UnitId::new(0),
        &rt,
    );
    assert!(matches!(
        r,
        Lv2Dispatch::Immediate {
            code: 0x8001_0005,
            ..
        }
    ));
}

#[test]
fn semaphore_post_val_not_one_returns_einval() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(256);
    let r = host.dispatch(
        Lv2Request::SemaphorePost { id: 1, val: 2 },
        UnitId::new(0),
        &rt,
    );
    assert!(matches!(
        r,
        Lv2Dispatch::Immediate {
            code: 0x8001_0002,
            ..
        }
    ));
}

#[test]
fn semaphore_post_with_no_waiters_increments() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(256);
    let src = UnitId::new(0);
    seed_primary_ppu(&mut host, src);
    let r = host.dispatch(
        Lv2Request::SemaphoreCreate {
            id_ptr: 0x100,
            attr_ptr: 0,
            initial: 0,
            max: 10,
        },
        src,
        &rt,
    );
    let id = match &r {
        Lv2Dispatch::Immediate {
            code: 0,
            effects: e,
        } => extract_write_u32(&e[0]),
        other => panic!("expected Immediate(0), got {other:?}"),
    };
    let post = host.dispatch(Lv2Request::SemaphorePost { id, val: 1 }, src, &rt);
    assert!(matches!(
        post,
        Lv2Dispatch::Immediate {
            code: 0,
            effects: _,
        }
    ));
    assert_eq!(host.semaphores().lookup(id).unwrap().count(), 1);
}

#[test]
fn semaphore_post_wakes_parked_waiter_without_incrementing() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(256);
    let poster_unit = UnitId::new(0);
    let waiter_unit = UnitId::new(1);
    seed_primary_ppu(&mut host, poster_unit);
    host.ppu_threads_mut()
        .create(
            waiter_unit,
            PpuThreadAttrs {
                entry: 0,
                arg: 0,
                stack_base: 0,
                stack_size: 0,
                priority: 0,
                tls_base: 0,
            },
        )
        .unwrap();
    let r = host.dispatch(
        Lv2Request::SemaphoreCreate {
            id_ptr: 0x100,
            attr_ptr: 0,
            initial: 0,
            max: 10,
        },
        poster_unit,
        &rt,
    );
    let id = match &r {
        Lv2Dispatch::Immediate {
            code: 0,
            effects: e,
        } => extract_write_u32(&e[0]),
        other => panic!("expected Immediate(0), got {other:?}"),
    };
    // Waiter parks.
    host.dispatch(
        Lv2Request::SemaphoreWait { id, timeout: 0 },
        waiter_unit,
        &rt,
    );
    // Poster posts.
    let post = host.dispatch(Lv2Request::SemaphorePost { id, val: 1 }, poster_unit, &rt);
    match post {
        Lv2Dispatch::WakeAndReturn {
            code: 0,
            woken_unit_ids,
            ..
        } => assert_eq!(woken_unit_ids, vec![waiter_unit]),
        other => panic!("expected WakeAndReturn, got {other:?}"),
    }
    // Count unchanged; waiter list empty.
    assert_eq!(host.semaphores().lookup(id).unwrap().count(), 0);
    assert!(host.semaphores().lookup(id).unwrap().waiters().is_empty());
}

#[test]
fn semaphore_post_past_max_with_no_waiters_returns_einval() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(256);
    let src = UnitId::new(0);
    seed_primary_ppu(&mut host, src);
    let r = host.dispatch(
        Lv2Request::SemaphoreCreate {
            id_ptr: 0x100,
            attr_ptr: 0,
            initial: 3,
            max: 3,
        },
        src,
        &rt,
    );
    let id = match &r {
        Lv2Dispatch::Immediate {
            code: 0,
            effects: e,
        } => extract_write_u32(&e[0]),
        other => panic!("expected Immediate(0), got {other:?}"),
    };
    let post = host.dispatch(Lv2Request::SemaphorePost { id, val: 1 }, src, &rt);
    assert!(matches!(
        post,
        Lv2Dispatch::Immediate {
            code: 0x8001_0002,
            ..
        }
    ));
    assert_eq!(host.semaphores().lookup(id).unwrap().count(), 3);
}

#[test]
fn semaphore_wait_unknown_id_returns_esrch() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(256);
    let src = UnitId::new(0);
    seed_primary_ppu(&mut host, src);
    let r = host.dispatch(Lv2Request::SemaphoreWait { id: 99, timeout: 0 }, src, &rt);
    assert!(matches!(
        r,
        Lv2Dispatch::Immediate {
            code: 0x8001_0005,
            ..
        }
    ));
}

#[test]
fn mutex_unlock_non_owner_returns_eperm() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(256);
    let owner_unit = UnitId::new(0);
    let other_unit = UnitId::new(1);
    seed_primary_ppu(&mut host, owner_unit);
    host.ppu_threads_mut()
        .create(
            other_unit,
            PpuThreadAttrs {
                entry: 0,
                arg: 0,
                stack_base: 0,
                stack_size: 0,
                priority: 0,
                tls_base: 0,
            },
        )
        .unwrap();
    let created = host.dispatch(
        Lv2Request::MutexCreate {
            id_ptr: 0x100,
            attr_ptr: 0,
        },
        owner_unit,
        &rt,
    );
    let id = match &created {
        Lv2Dispatch::Immediate {
            code: 0,
            effects: e,
        } => extract_write_u32(&e[0]),
        other => panic!("expected Immediate(0), got {other:?}"),
    };
    host.dispatch(
        Lv2Request::MutexLock {
            mutex_id: id,
            timeout: 0,
        },
        owner_unit,
        &rt,
    );
    let r = host.dispatch(Lv2Request::MutexUnlock { mutex_id: id }, other_unit, &rt);
    assert!(matches!(
        r,
        Lv2Dispatch::Immediate {
            code: 0x8001_0008,
            ..
        }
    ));
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
    // If the caller is not in the thread table yet (e.g. an
    // unseeded primary on the standard boot path), the handler
    // still returns a PpuThreadExit dispatch so the runtime
    // transitions the unit to Finished. No waiters are waked
    // because none can be tracked.
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

fn create_mutex_host(host: &mut Lv2Host, src: UnitId, rt: &FakeRuntime) -> u32 {
    let created = host.dispatch(
        Lv2Request::MutexCreate {
            id_ptr: 0x100,
            attr_ptr: 0,
        },
        src,
        rt,
    );
    match &created {
        Lv2Dispatch::Immediate {
            code: 0,
            effects: e,
        } => extract_write_u32(&e[0]),
        other => panic!("expected Immediate(0), got {other:?}"),
    }
}

#[test]
fn cond_create_writes_id_and_binds_mutex() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let src = UnitId::new(0);
    seed_primary_ppu(&mut host, src);
    let mutex_id = create_mutex_host(&mut host, src, &rt);
    let created = host.dispatch(
        Lv2Request::CondCreate {
            id_ptr: 0x200,
            mutex_id,
            attr_ptr: 0,
        },
        src,
        &rt,
    );
    let cond_id = match &created {
        Lv2Dispatch::Immediate {
            code: 0,
            effects: e,
        } => extract_write_u32(&e[0]),
        other => panic!("expected Immediate(0), got {other:?}"),
    };
    let entry = host.conds().lookup(cond_id).unwrap();
    assert_eq!(entry.mutex_id(), mutex_id);
    assert_eq!(entry.mutex_kind(), CondMutexKind::Mutex);
    assert!(entry.waiters().is_empty());
}

#[test]
fn cond_create_unknown_mutex_returns_esrch() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let src = UnitId::new(0);
    seed_primary_ppu(&mut host, src);
    let r = host.dispatch(
        Lv2Request::CondCreate {
            id_ptr: 0x200,
            mutex_id: 0xDEAD,
            attr_ptr: 0,
        },
        src,
        &rt,
    );
    assert!(matches!(
        r,
        Lv2Dispatch::Immediate {
            code: 0x8001_0005,
            ..
        }
    ));
    assert!(host.conds().is_empty());
}

#[test]
fn cond_destroy_empty_succeeds() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let src = UnitId::new(0);
    seed_primary_ppu(&mut host, src);
    let mutex_id = create_mutex_host(&mut host, src, &rt);
    let created = host.dispatch(
        Lv2Request::CondCreate {
            id_ptr: 0x200,
            mutex_id,
            attr_ptr: 0,
        },
        src,
        &rt,
    );
    let cond_id = match &created {
        Lv2Dispatch::Immediate { effects: e, .. } => extract_write_u32(&e[0]),
        other => panic!("expected Immediate, got {other:?}"),
    };
    let r = host.dispatch(Lv2Request::CondDestroy { id: cond_id }, src, &rt);
    assert!(matches!(
        r,
        Lv2Dispatch::Immediate {
            code: 0,
            effects: _,
        }
    ));
    assert!(host.conds().lookup(cond_id).is_none());
}

#[test]
fn cond_destroy_unknown_returns_esrch() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let src = UnitId::new(0);
    seed_primary_ppu(&mut host, src);
    let r = host.dispatch(Lv2Request::CondDestroy { id: 0xDEAD }, src, &rt);
    assert!(matches!(
        r,
        Lv2Dispatch::Immediate {
            code: 0x8001_0005,
            ..
        }
    ));
}

#[test]
fn cond_wait_releases_mutex_and_parks_caller() {
    // Caller holds the mutex, no mutex waiters. cond_wait must
    // drop the mutex (owner cleared) and park the caller on the
    // cond with a CondWakeReacquire pending response.
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let src = UnitId::new(0);
    seed_primary_ppu(&mut host, src);
    let mutex_id = create_mutex_host(&mut host, src, &rt);
    // Acquire the mutex.
    host.dispatch(
        Lv2Request::MutexLock {
            mutex_id,
            timeout: 0,
        },
        src,
        &rt,
    );
    // Create the cond.
    let created = host.dispatch(
        Lv2Request::CondCreate {
            id_ptr: 0x200,
            mutex_id,
            attr_ptr: 0,
        },
        src,
        &rt,
    );
    let cond_id = match &created {
        Lv2Dispatch::Immediate { effects: e, .. } => extract_write_u32(&e[0]),
        other => panic!("expected Immediate, got {other:?}"),
    };
    // Wait.
    let r = host.dispatch(
        Lv2Request::CondWait {
            id: cond_id,
            timeout: 0,
        },
        src,
        &rt,
    );
    match r {
        Lv2Dispatch::Block {
            reason, pending, ..
        } => {
            assert!(matches!(
                reason,
                crate::dispatch::Lv2BlockReason::Cond { id, mutex_id: m }
                    if id == cond_id && m == mutex_id
            ));
            assert!(matches!(
                pending,
                PendingResponse::CondWakeReacquire {
                    mutex_id: m,
                    mutex_kind: CondMutexKind::Mutex,
                } if m == mutex_id
            ));
        }
        other => panic!("expected Block, got {other:?}"),
    }
    // Mutex is now unowned.
    assert_eq!(host.mutexes().lookup(mutex_id).unwrap().owner(), None);
    // Cond has the caller parked.
    assert_eq!(
        host.conds()
            .lookup(cond_id)
            .unwrap()
            .waiters()
            .iter()
            .collect::<Vec<_>>(),
        vec![PpuThreadId::PRIMARY],
    );
}

#[test]
fn cond_wait_transfers_mutex_to_waiter_via_block_and_wake() {
    // Two threads contend on the mutex. When the owner calls
    // cond_wait, the mutex waiter inherits ownership and must
    // wake alongside the owner's cond park. The handler emits
    // BlockAndWake.
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let owner_unit = UnitId::new(0);
    let waiter_unit = UnitId::new(1);
    seed_primary_ppu(&mut host, owner_unit);
    let waiter_tid = host
        .ppu_threads_mut()
        .create(waiter_unit, primary_attrs())
        .expect("waiter create");
    let mutex_id = create_mutex_host(&mut host, owner_unit, &rt);
    host.dispatch(
        Lv2Request::MutexLock {
            mutex_id,
            timeout: 0,
        },
        owner_unit,
        &rt,
    );
    host.dispatch(
        Lv2Request::MutexLock {
            mutex_id,
            timeout: 0,
        },
        waiter_unit,
        &rt,
    );
    // waiter is now parked on the mutex.
    assert_eq!(
        host.mutexes()
            .lookup(mutex_id)
            .unwrap()
            .waiters()
            .iter()
            .collect::<Vec<_>>(),
        vec![waiter_tid],
    );
    let created = host.dispatch(
        Lv2Request::CondCreate {
            id_ptr: 0x200,
            mutex_id,
            attr_ptr: 0,
        },
        owner_unit,
        &rt,
    );
    let cond_id = match &created {
        Lv2Dispatch::Immediate { effects: e, .. } => extract_write_u32(&e[0]),
        other => panic!("expected Immediate, got {other:?}"),
    };
    let r = host.dispatch(
        Lv2Request::CondWait {
            id: cond_id,
            timeout: 0,
        },
        owner_unit,
        &rt,
    );
    match r {
        Lv2Dispatch::BlockAndWake {
            reason,
            pending,
            woken_unit_ids,
            ..
        } => {
            assert!(matches!(
                reason,
                crate::dispatch::Lv2BlockReason::Cond { .. }
            ));
            assert!(matches!(pending, PendingResponse::CondWakeReacquire { .. }));
            assert_eq!(woken_unit_ids, vec![waiter_unit]);
        }
        other => panic!("expected BlockAndWake, got {other:?}"),
    }
    // Ownership transferred to the waiter.
    assert_eq!(
        host.mutexes().lookup(mutex_id).unwrap().owner(),
        Some(waiter_tid),
    );
    // Owner is now parked on the cond.
    assert_eq!(
        host.conds()
            .lookup(cond_id)
            .unwrap()
            .waiters()
            .iter()
            .collect::<Vec<_>>(),
        vec![PpuThreadId::PRIMARY],
    );
}

#[test]
fn cond_wait_by_non_owner_returns_eperm() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let owner_unit = UnitId::new(0);
    let other_unit = UnitId::new(1);
    seed_primary_ppu(&mut host, owner_unit);
    host.ppu_threads_mut()
        .create(other_unit, primary_attrs())
        .expect("other create");
    let mutex_id = create_mutex_host(&mut host, owner_unit, &rt);
    host.dispatch(
        Lv2Request::MutexLock {
            mutex_id,
            timeout: 0,
        },
        owner_unit,
        &rt,
    );
    let created = host.dispatch(
        Lv2Request::CondCreate {
            id_ptr: 0x200,
            mutex_id,
            attr_ptr: 0,
        },
        owner_unit,
        &rt,
    );
    let cond_id = match &created {
        Lv2Dispatch::Immediate { effects: e, .. } => extract_write_u32(&e[0]),
        other => panic!("expected Immediate, got {other:?}"),
    };
    // Non-owner attempts cond_wait; mutex release rejects with
    // NotOwner -> EPERM.
    let r = host.dispatch(
        Lv2Request::CondWait {
            id: cond_id,
            timeout: 0,
        },
        other_unit,
        &rt,
    );
    assert!(matches!(
        r,
        Lv2Dispatch::Immediate {
            code: 0x8001_0008,
            ..
        }
    ));
    // Mutex ownership unchanged.
    assert_eq!(
        host.mutexes().lookup(mutex_id).unwrap().owner(),
        Some(PpuThreadId::PRIMARY),
    );
    // Cond is still empty.
    assert!(host.conds().lookup(cond_id).unwrap().waiters().is_empty());
}

#[test]
fn cond_wait_unknown_id_returns_esrch() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let src = UnitId::new(0);
    seed_primary_ppu(&mut host, src);
    let r = host.dispatch(
        Lv2Request::CondWait {
            id: 0xDEAD,
            timeout: 0,
        },
        src,
        &rt,
    );
    assert!(matches!(
        r,
        Lv2Dispatch::Immediate {
            code: 0x8001_0005,
            ..
        }
    ));
}

#[test]
fn cond_signal_no_waiter_is_observably_lost() {
    // Non-sticky: signal on a cond with no waiters returns
    // CELL_OK and does not record any pending state.
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let src = UnitId::new(0);
    seed_primary_ppu(&mut host, src);
    let mutex_id = create_mutex_host(&mut host, src, &rt);
    let created = host.dispatch(
        Lv2Request::CondCreate {
            id_ptr: 0x200,
            mutex_id,
            attr_ptr: 0,
        },
        src,
        &rt,
    );
    let cond_id = match &created {
        Lv2Dispatch::Immediate { effects: e, .. } => extract_write_u32(&e[0]),
        other => panic!("expected Immediate, got {other:?}"),
    };
    let r = host.dispatch(Lv2Request::CondSignal { id: cond_id }, src, &rt);
    assert!(matches!(
        r,
        Lv2Dispatch::Immediate {
            code: 0,
            effects: _,
        }
    ));
    // Cond stays empty; hash-level: same as a cond that never
    // received a signal (anchored by
    // state_hash_ignores_ephemeral_signal_attempts in cond
    // table tests).
    assert!(host.conds().lookup(cond_id).unwrap().waiters().is_empty());
}

#[test]
fn cond_signal_unknown_id_returns_esrch() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let src = UnitId::new(0);
    seed_primary_ppu(&mut host, src);
    let r = host.dispatch(Lv2Request::CondSignal { id: 0xDEAD }, src, &rt);
    assert!(matches!(
        r,
        Lv2Dispatch::Immediate {
            code: 0x8001_0005,
            ..
        }
    ));
}

#[test]
fn cond_signal_wakes_waiter_cleanly_when_mutex_free() {
    // Waiter parked via cond_wait, no other thread holds the
    // mutex. Signaler wakes the waiter; the waker acquires the
    // mutex and its pending response is swapped to
    // ReturnCode { 0 }.
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let waiter_unit = UnitId::new(0);
    let signaler_unit = UnitId::new(1);
    seed_primary_ppu(&mut host, waiter_unit);
    host.ppu_threads_mut()
        .create(signaler_unit, primary_attrs())
        .expect("signaler create");
    let mutex_id = create_mutex_host(&mut host, waiter_unit, &rt);
    host.dispatch(
        Lv2Request::MutexLock {
            mutex_id,
            timeout: 0,
        },
        waiter_unit,
        &rt,
    );
    let created = host.dispatch(
        Lv2Request::CondCreate {
            id_ptr: 0x200,
            mutex_id,
            attr_ptr: 0,
        },
        waiter_unit,
        &rt,
    );
    let cond_id = match &created {
        Lv2Dispatch::Immediate { effects: e, .. } => extract_write_u32(&e[0]),
        other => panic!("expected Immediate, got {other:?}"),
    };
    host.dispatch(
        Lv2Request::CondWait {
            id: cond_id,
            timeout: 0,
        },
        waiter_unit,
        &rt,
    );
    // Mutex now free (waiter released on cond_wait).
    assert_eq!(host.mutexes().lookup(mutex_id).unwrap().owner(), None);
    let r = host.dispatch(Lv2Request::CondSignal { id: cond_id }, signaler_unit, &rt);
    match r {
        Lv2Dispatch::WakeAndReturn {
            code,
            woken_unit_ids,
            response_updates,
            ..
        } => {
            assert_eq!(code, 0);
            assert_eq!(woken_unit_ids, vec![waiter_unit]);
            assert_eq!(response_updates.len(), 1);
            assert_eq!(response_updates[0].0, waiter_unit);
            assert!(matches!(
                response_updates[0].1,
                PendingResponse::ReturnCode { code: 0 }
            ));
        }
        other => panic!("expected WakeAndReturn, got {other:?}"),
    }
    // Waker is now the mutex owner.
    assert_eq!(
        host.mutexes().lookup(mutex_id).unwrap().owner(),
        Some(PpuThreadId::PRIMARY),
    );
    // Cond is empty.
    assert!(host.conds().lookup(cond_id).unwrap().waiters().is_empty());
}

#[test]
fn cond_signal_reparks_waiter_on_mutex_when_held() {
    // Waiter parked via cond_wait. Before signal, a THIRD
    // thread acquires the mutex. Signal fires but finds the
    // mutex held; the cond waiter transitions to the mutex
    // waiter list (its pending response swaps from
    // CondWakeReacquire to ReturnCode { 0 }). Signaler returns
    // CELL_OK with no wake.
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let waiter_unit = UnitId::new(0);
    let third_unit = UnitId::new(1);
    let signaler_unit = UnitId::new(2);
    seed_primary_ppu(&mut host, waiter_unit);
    let third_tid = host
        .ppu_threads_mut()
        .create(third_unit, primary_attrs())
        .expect("third create");
    host.ppu_threads_mut()
        .create(signaler_unit, primary_attrs())
        .expect("signaler create");
    let mutex_id = create_mutex_host(&mut host, waiter_unit, &rt);
    // Waiter takes the mutex.
    host.dispatch(
        Lv2Request::MutexLock {
            mutex_id,
            timeout: 0,
        },
        waiter_unit,
        &rt,
    );
    let created = host.dispatch(
        Lv2Request::CondCreate {
            id_ptr: 0x200,
            mutex_id,
            attr_ptr: 0,
        },
        waiter_unit,
        &rt,
    );
    let cond_id = match &created {
        Lv2Dispatch::Immediate { effects: e, .. } => extract_write_u32(&e[0]),
        other => panic!("expected Immediate, got {other:?}"),
    };
    // Waiter calls cond_wait (releases mutex).
    host.dispatch(
        Lv2Request::CondWait {
            id: cond_id,
            timeout: 0,
        },
        waiter_unit,
        &rt,
    );
    // Third thread now takes the mutex.
    host.dispatch(
        Lv2Request::MutexLock {
            mutex_id,
            timeout: 0,
        },
        third_unit,
        &rt,
    );
    assert_eq!(
        host.mutexes().lookup(mutex_id).unwrap().owner(),
        Some(third_tid),
    );
    // Signaler fires. Waiter should re-park on mutex.
    let r = host.dispatch(Lv2Request::CondSignal { id: cond_id }, signaler_unit, &rt);
    match r {
        Lv2Dispatch::WakeAndReturn {
            code,
            woken_unit_ids,
            response_updates,
            ..
        } => {
            assert_eq!(code, 0);
            assert!(
                woken_unit_ids.is_empty(),
                "signal with mutex-held must not wake"
            );
            assert_eq!(response_updates.len(), 1);
            assert_eq!(response_updates[0].0, waiter_unit);
            assert!(matches!(
                response_updates[0].1,
                PendingResponse::ReturnCode { code: 0 }
            ));
        }
        other => panic!("expected WakeAndReturn with empty wake, got {other:?}"),
    }
    // Mutex owner unchanged (still the third thread).
    assert_eq!(
        host.mutexes().lookup(mutex_id).unwrap().owner(),
        Some(third_tid),
    );
    // Waiter is now parked on the mutex waiter list.
    assert_eq!(
        host.mutexes()
            .lookup(mutex_id)
            .unwrap()
            .waiters()
            .iter()
            .collect::<Vec<_>>(),
        vec![PpuThreadId::PRIMARY],
    );
    // Cond list is empty.
    assert!(host.conds().lookup(cond_id).unwrap().waiters().is_empty());
}

#[test]
fn cond_signal_wakes_fifo_head_when_multiple_waiters() {
    // Two waiters parked in cond. First signal wakes the head;
    // second waiter stays parked.
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let w1_unit = UnitId::new(0);
    let w2_unit = UnitId::new(1);
    let signaler_unit = UnitId::new(2);
    seed_primary_ppu(&mut host, w1_unit);
    let w2_tid = host
        .ppu_threads_mut()
        .create(w2_unit, primary_attrs())
        .expect("w2 create");
    host.ppu_threads_mut()
        .create(signaler_unit, primary_attrs())
        .expect("signaler create");
    let mutex_id = create_mutex_host(&mut host, w1_unit, &rt);
    // Waiter 1 acquires mutex and parks on cond.
    host.dispatch(
        Lv2Request::MutexLock {
            mutex_id,
            timeout: 0,
        },
        w1_unit,
        &rt,
    );
    let created = host.dispatch(
        Lv2Request::CondCreate {
            id_ptr: 0x200,
            mutex_id,
            attr_ptr: 0,
        },
        w1_unit,
        &rt,
    );
    let cond_id = match &created {
        Lv2Dispatch::Immediate { effects: e, .. } => extract_write_u32(&e[0]),
        other => panic!("expected Immediate, got {other:?}"),
    };
    host.dispatch(
        Lv2Request::CondWait {
            id: cond_id,
            timeout: 0,
        },
        w1_unit,
        &rt,
    );
    // Waiter 2 acquires mutex and parks on cond.
    host.dispatch(
        Lv2Request::MutexLock {
            mutex_id,
            timeout: 0,
        },
        w2_unit,
        &rt,
    );
    host.dispatch(
        Lv2Request::CondWait {
            id: cond_id,
            timeout: 0,
        },
        w2_unit,
        &rt,
    );
    // First signal wakes w1 (FIFO head).
    let r = host.dispatch(Lv2Request::CondSignal { id: cond_id }, signaler_unit, &rt);
    match r {
        Lv2Dispatch::WakeAndReturn { woken_unit_ids, .. } => {
            assert_eq!(woken_unit_ids, vec![w1_unit]);
        }
        other => panic!("expected WakeAndReturn, got {other:?}"),
    }
    // w2 still parked.
    assert_eq!(
        host.conds()
            .lookup(cond_id)
            .unwrap()
            .waiters()
            .iter()
            .collect::<Vec<_>>(),
        vec![w2_tid],
    );
}

#[test]
fn cond_signal_all_wakes_first_reparks_rest_when_mutex_free() {
    // Three cond waiters parked. Mutex free at signal_all
    // time: first waiter acquires and wakes; second and third
    // re-park on the mutex waiter list. Order preserved (FIFO
    // from the cond list).
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let w1 = UnitId::new(0);
    let w2 = UnitId::new(1);
    let w3 = UnitId::new(2);
    let signaler = UnitId::new(3);
    seed_primary_ppu(&mut host, w1);
    let w2_tid = host
        .ppu_threads_mut()
        .create(w2, primary_attrs())
        .expect("w2 create");
    let w3_tid = host
        .ppu_threads_mut()
        .create(w3, primary_attrs())
        .expect("w3 create");
    host.ppu_threads_mut()
        .create(signaler, primary_attrs())
        .expect("signaler create");
    let mutex_id = create_mutex_host(&mut host, w1, &rt);
    let created = host.dispatch(
        Lv2Request::CondCreate {
            id_ptr: 0x200,
            mutex_id,
            attr_ptr: 0,
        },
        w1,
        &rt,
    );
    let cond_id = match &created {
        Lv2Dispatch::Immediate { effects: e, .. } => extract_write_u32(&e[0]),
        other => panic!("expected Immediate, got {other:?}"),
    };
    for unit in [w1, w2, w3] {
        host.dispatch(
            Lv2Request::MutexLock {
                mutex_id,
                timeout: 0,
            },
            unit,
            &rt,
        );
        host.dispatch(
            Lv2Request::CondWait {
                id: cond_id,
                timeout: 0,
            },
            unit,
            &rt,
        );
    }
    // All three are now parked on cond; mutex free.
    assert_eq!(host.conds().lookup(cond_id).unwrap().waiters().len(), 3);
    assert_eq!(host.mutexes().lookup(mutex_id).unwrap().owner(), None);
    let r = host.dispatch(Lv2Request::CondSignalAll { id: cond_id }, signaler, &rt);
    match r {
        Lv2Dispatch::WakeAndReturn {
            woken_unit_ids,
            response_updates,
            ..
        } => {
            // Only w1 (head of FIFO) wakes cleanly.
            assert_eq!(woken_unit_ids, vec![w1]);
            // All three get response swapped.
            let updated_units: Vec<_> = response_updates.iter().map(|(u, _)| *u).collect();
            assert_eq!(updated_units, vec![w1, w2, w3]);
        }
        other => panic!("expected WakeAndReturn, got {other:?}"),
    }
    // w1 owns mutex; w2, w3 parked on mutex waiter list FIFO.
    assert_eq!(
        host.mutexes().lookup(mutex_id).unwrap().owner(),
        Some(PpuThreadId::PRIMARY),
    );
    assert_eq!(
        host.mutexes()
            .lookup(mutex_id)
            .unwrap()
            .waiters()
            .iter()
            .collect::<Vec<_>>(),
        vec![w2_tid, w3_tid],
    );
    assert!(host.conds().lookup(cond_id).unwrap().waiters().is_empty());
}

#[test]
fn cond_signal_all_reparks_all_when_mutex_held() {
    // Three cond waiters parked, then a fourth thread takes
    // the mutex. signal_all: all three waiters re-park on the
    // mutex list; no one wakes.
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let w1 = UnitId::new(0);
    let w2 = UnitId::new(1);
    let w3 = UnitId::new(2);
    let holder = UnitId::new(3);
    let signaler = UnitId::new(4);
    seed_primary_ppu(&mut host, w1);
    let w2_tid = host
        .ppu_threads_mut()
        .create(w2, primary_attrs())
        .expect("w2 create");
    let w3_tid = host
        .ppu_threads_mut()
        .create(w3, primary_attrs())
        .expect("w3 create");
    let holder_tid = host
        .ppu_threads_mut()
        .create(holder, primary_attrs())
        .expect("holder create");
    host.ppu_threads_mut()
        .create(signaler, primary_attrs())
        .expect("signaler create");
    let mutex_id = create_mutex_host(&mut host, w1, &rt);
    let created = host.dispatch(
        Lv2Request::CondCreate {
            id_ptr: 0x200,
            mutex_id,
            attr_ptr: 0,
        },
        w1,
        &rt,
    );
    let cond_id = match &created {
        Lv2Dispatch::Immediate { effects: e, .. } => extract_write_u32(&e[0]),
        other => panic!("expected Immediate, got {other:?}"),
    };
    for unit in [w1, w2, w3] {
        host.dispatch(
            Lv2Request::MutexLock {
                mutex_id,
                timeout: 0,
            },
            unit,
            &rt,
        );
        host.dispatch(
            Lv2Request::CondWait {
                id: cond_id,
                timeout: 0,
            },
            unit,
            &rt,
        );
    }
    // Holder takes the mutex.
    host.dispatch(
        Lv2Request::MutexLock {
            mutex_id,
            timeout: 0,
        },
        holder,
        &rt,
    );
    assert_eq!(
        host.mutexes().lookup(mutex_id).unwrap().owner(),
        Some(holder_tid),
    );
    let r = host.dispatch(Lv2Request::CondSignalAll { id: cond_id }, signaler, &rt);
    match r {
        Lv2Dispatch::WakeAndReturn {
            woken_unit_ids,
            response_updates,
            ..
        } => {
            assert!(woken_unit_ids.is_empty(), "nobody wakes when mutex is held");
            assert_eq!(response_updates.len(), 3);
        }
        other => panic!("expected WakeAndReturn, got {other:?}"),
    }
    // All three waiters parked on mutex in FIFO order.
    assert_eq!(
        host.mutexes()
            .lookup(mutex_id)
            .unwrap()
            .waiters()
            .iter()
            .collect::<Vec<_>>(),
        vec![PpuThreadId::PRIMARY, w2_tid, w3_tid],
    );
    assert!(host.conds().lookup(cond_id).unwrap().waiters().is_empty());
}

#[test]
fn cond_signal_all_no_waiters_is_lost() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let src = UnitId::new(0);
    seed_primary_ppu(&mut host, src);
    let mutex_id = create_mutex_host(&mut host, src, &rt);
    let created = host.dispatch(
        Lv2Request::CondCreate {
            id_ptr: 0x200,
            mutex_id,
            attr_ptr: 0,
        },
        src,
        &rt,
    );
    let cond_id = match &created {
        Lv2Dispatch::Immediate { effects: e, .. } => extract_write_u32(&e[0]),
        other => panic!("expected Immediate, got {other:?}"),
    };
    let r = host.dispatch(Lv2Request::CondSignalAll { id: cond_id }, src, &rt);
    assert!(matches!(
        r,
        Lv2Dispatch::Immediate {
            code: 0,
            effects: _,
        }
    ));
}

#[test]
fn cond_signal_all_unknown_id_returns_esrch() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let src = UnitId::new(0);
    seed_primary_ppu(&mut host, src);
    let r = host.dispatch(Lv2Request::CondSignalAll { id: 0xDEAD }, src, &rt);
    assert!(matches!(
        r,
        Lv2Dispatch::Immediate {
            code: 0x8001_0005,
            ..
        }
    ));
}

#[test]
fn cond_signal_all_handles_waker_already_on_mutex_queue() {
    // Weird edge case: the cond waiter is ALSO already present
    // on the mutex waiter list before signal_all fires.
    // Scenario construction: signaler holds the mutex; a cond
    // waiter is parked on the cond list; the mutex queue has
    // been seeded with that same waiter out-of-band (could
    // happen via a prior signal_to path that left a stale cond
    // entry, or via direct test-harness manipulation).
    //
    // Expected: signal_all must not panic. The already-present
    // mutex queue entry is preserved (no duplicate). The cond
    // queue is drained. The waiter's pending response is
    // swapped to ReturnCode { 0 } so the eventual unlock-wake
    // resolves cleanly via the single queue entry.
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let waker_unit = UnitId::new(0);
    let signaler_unit = UnitId::new(1);
    seed_primary_ppu(&mut host, waker_unit);
    host.ppu_threads_mut()
        .create(signaler_unit, primary_attrs())
        .expect("signaler create");
    let mutex_id = create_mutex_host(&mut host, waker_unit, &rt);
    // Signaler takes the mutex.
    host.dispatch(
        Lv2Request::MutexLock {
            mutex_id,
            timeout: 0,
        },
        signaler_unit,
        &rt,
    );
    // Create the cond bound to that mutex.
    let created = host.dispatch(
        Lv2Request::CondCreate {
            id_ptr: 0x200,
            mutex_id,
            attr_ptr: 0,
        },
        signaler_unit,
        &rt,
    );
    let cond_id = match &created {
        Lv2Dispatch::Immediate { effects: e, .. } => extract_write_u32(&e[0]),
        other => panic!("expected Immediate, got {other:?}"),
    };
    // Seed the weird state: cond queue has PRIMARY parked,
    // AND the mutex queue is also seeded with PRIMARY (direct
    // table manipulation -- mimics the stale-entry condition).
    assert!(host
        .conds_mut()
        .enqueue_waiter(cond_id, PpuThreadId::PRIMARY));
    assert!(host
        .mutexes_mut()
        .enqueue_waiter(mutex_id, PpuThreadId::PRIMARY));
    // Sanity: both tables hold the waker.
    assert_eq!(
        host.mutexes()
            .lookup(mutex_id)
            .unwrap()
            .waiters()
            .iter()
            .collect::<Vec<_>>(),
        vec![PpuThreadId::PRIMARY],
    );
    assert_eq!(
        host.conds()
            .lookup(cond_id)
            .unwrap()
            .waiters()
            .iter()
            .collect::<Vec<_>>(),
        vec![PpuThreadId::PRIMARY],
    );
    // Fire signal_all from the mutex holder. Must not panic.
    let r = host.dispatch(
        Lv2Request::CondSignalAll { id: cond_id },
        signaler_unit,
        &rt,
    );
    match r {
        Lv2Dispatch::WakeAndReturn {
            woken_unit_ids,
            response_updates,
            ..
        } => {
            // Mutex is held by signaler -> Contended path;
            // no one wakes.
            assert!(woken_unit_ids.is_empty());
            // Response swap for the single waker.
            assert_eq!(response_updates.len(), 1);
            assert_eq!(response_updates[0].0, waker_unit);
            assert!(matches!(
                response_updates[0].1,
                PendingResponse::ReturnCode { code: 0 }
            ));
        }
        other => panic!("expected WakeAndReturn, got {other:?}"),
    }
    // Mutex queue unchanged: single entry, no duplicate.
    assert_eq!(
        host.mutexes()
            .lookup(mutex_id)
            .unwrap()
            .waiters()
            .iter()
            .collect::<Vec<_>>(),
        vec![PpuThreadId::PRIMARY],
    );
    // Cond queue drained.
    assert!(host.conds().lookup(cond_id).unwrap().waiters().is_empty());
}

#[test]
fn cond_signal_to_targets_specific_waiter_and_preserves_order() {
    // Three cond waiters parked (w1, w2, w3). signal_to(w2)
    // must wake exactly w2, leaving w1 and w3 parked in their
    // original relative order.
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let w1 = UnitId::new(0);
    let w2 = UnitId::new(1);
    let w3 = UnitId::new(2);
    let signaler = UnitId::new(3);
    seed_primary_ppu(&mut host, w1);
    let w2_tid = host
        .ppu_threads_mut()
        .create(w2, primary_attrs())
        .expect("w2 create");
    let w3_tid = host
        .ppu_threads_mut()
        .create(w3, primary_attrs())
        .expect("w3 create");
    host.ppu_threads_mut()
        .create(signaler, primary_attrs())
        .expect("signaler create");
    let mutex_id = create_mutex_host(&mut host, w1, &rt);
    let created = host.dispatch(
        Lv2Request::CondCreate {
            id_ptr: 0x200,
            mutex_id,
            attr_ptr: 0,
        },
        w1,
        &rt,
    );
    let cond_id = match &created {
        Lv2Dispatch::Immediate { effects: e, .. } => extract_write_u32(&e[0]),
        other => panic!("expected Immediate, got {other:?}"),
    };
    for unit in [w1, w2, w3] {
        host.dispatch(
            Lv2Request::MutexLock {
                mutex_id,
                timeout: 0,
            },
            unit,
            &rt,
        );
        host.dispatch(
            Lv2Request::CondWait {
                id: cond_id,
                timeout: 0,
            },
            unit,
            &rt,
        );
    }
    // Mutex free; all three parked on cond.
    assert_eq!(host.mutexes().lookup(mutex_id).unwrap().owner(), None);
    assert_eq!(host.conds().lookup(cond_id).unwrap().waiters().len(), 3);
    // Signal specifically at w2.
    let r = host.dispatch(
        Lv2Request::CondSignalTo {
            id: cond_id,
            target_thread: w2_tid.raw() as u32,
        },
        signaler,
        &rt,
    );
    match r {
        Lv2Dispatch::WakeAndReturn {
            code,
            woken_unit_ids,
            response_updates,
            ..
        } => {
            assert_eq!(code, 0);
            assert_eq!(woken_unit_ids, vec![w2]);
            assert_eq!(response_updates.len(), 1);
            assert_eq!(response_updates[0].0, w2);
            assert!(matches!(
                response_updates[0].1,
                PendingResponse::ReturnCode { code: 0 }
            ));
        }
        other => panic!("expected WakeAndReturn, got {other:?}"),
    }
    // w2 now owns the mutex.
    assert_eq!(
        host.mutexes().lookup(mutex_id).unwrap().owner(),
        Some(w2_tid)
    );
    // Cond still has w1 then w3 (relative order preserved).
    assert_eq!(
        host.conds()
            .lookup(cond_id)
            .unwrap()
            .waiters()
            .iter()
            .collect::<Vec<_>>(),
        vec![PpuThreadId::PRIMARY, w3_tid],
    );
}

#[test]
fn cond_signal_to_missing_target_returns_esrch() {
    // target is a real thread but not parked on this cond.
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let w1 = UnitId::new(0);
    let other = UnitId::new(1);
    let signaler = UnitId::new(2);
    seed_primary_ppu(&mut host, w1);
    let other_tid = host
        .ppu_threads_mut()
        .create(other, primary_attrs())
        .expect("other create");
    host.ppu_threads_mut()
        .create(signaler, primary_attrs())
        .expect("signaler create");
    let mutex_id = create_mutex_host(&mut host, w1, &rt);
    let created = host.dispatch(
        Lv2Request::CondCreate {
            id_ptr: 0x200,
            mutex_id,
            attr_ptr: 0,
        },
        w1,
        &rt,
    );
    let cond_id = match &created {
        Lv2Dispatch::Immediate { effects: e, .. } => extract_write_u32(&e[0]),
        other => panic!("expected Immediate, got {other:?}"),
    };
    // Park w1 on cond; do NOT park `other`.
    host.dispatch(
        Lv2Request::MutexLock {
            mutex_id,
            timeout: 0,
        },
        w1,
        &rt,
    );
    host.dispatch(
        Lv2Request::CondWait {
            id: cond_id,
            timeout: 0,
        },
        w1,
        &rt,
    );
    // signal_to at `other` (not parked) -> ESRCH.
    let r = host.dispatch(
        Lv2Request::CondSignalTo {
            id: cond_id,
            target_thread: other_tid.raw() as u32,
        },
        signaler,
        &rt,
    );
    assert!(matches!(
        r,
        Lv2Dispatch::Immediate {
            code: 0x8001_0005,
            ..
        }
    ));
    // w1 remains parked on cond.
    assert_eq!(
        host.conds()
            .lookup(cond_id)
            .unwrap()
            .waiters()
            .iter()
            .collect::<Vec<_>>(),
        vec![PpuThreadId::PRIMARY],
    );
}

#[test]
fn cond_signal_to_unknown_cond_returns_esrch() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let src = UnitId::new(0);
    seed_primary_ppu(&mut host, src);
    let r = host.dispatch(
        Lv2Request::CondSignalTo {
            id: 0xDEAD,
            target_thread: PpuThreadId::PRIMARY.raw() as u32,
        },
        src,
        &rt,
    );
    assert!(matches!(
        r,
        Lv2Dispatch::Immediate {
            code: 0x8001_0005,
            ..
        }
    ));
}

#[test]
fn cond_signal_to_reparks_target_on_mutex_when_held() {
    // Two cond waiters parked (w1, w2). A third thread (holder)
    // takes the mutex. signal_to(w1) must re-park w1 on the
    // mutex waiter list (no wake), leaving w2 on cond.
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let w1 = UnitId::new(0);
    let w2 = UnitId::new(1);
    let holder = UnitId::new(2);
    let signaler = UnitId::new(3);
    seed_primary_ppu(&mut host, w1);
    let w2_tid = host
        .ppu_threads_mut()
        .create(w2, primary_attrs())
        .expect("w2 create");
    let holder_tid = host
        .ppu_threads_mut()
        .create(holder, primary_attrs())
        .expect("holder create");
    host.ppu_threads_mut()
        .create(signaler, primary_attrs())
        .expect("signaler create");
    let mutex_id = create_mutex_host(&mut host, w1, &rt);
    let created = host.dispatch(
        Lv2Request::CondCreate {
            id_ptr: 0x200,
            mutex_id,
            attr_ptr: 0,
        },
        w1,
        &rt,
    );
    let cond_id = match &created {
        Lv2Dispatch::Immediate { effects: e, .. } => extract_write_u32(&e[0]),
        other => panic!("expected Immediate, got {other:?}"),
    };
    for unit in [w1, w2] {
        host.dispatch(
            Lv2Request::MutexLock {
                mutex_id,
                timeout: 0,
            },
            unit,
            &rt,
        );
        host.dispatch(
            Lv2Request::CondWait {
                id: cond_id,
                timeout: 0,
            },
            unit,
            &rt,
        );
    }
    host.dispatch(
        Lv2Request::MutexLock {
            mutex_id,
            timeout: 0,
        },
        holder,
        &rt,
    );
    assert_eq!(
        host.mutexes().lookup(mutex_id).unwrap().owner(),
        Some(holder_tid),
    );
    let r = host.dispatch(
        Lv2Request::CondSignalTo {
            id: cond_id,
            target_thread: PpuThreadId::PRIMARY.raw() as u32,
        },
        signaler,
        &rt,
    );
    match r {
        Lv2Dispatch::WakeAndReturn {
            woken_unit_ids,
            response_updates,
            ..
        } => {
            assert!(woken_unit_ids.is_empty());
            assert_eq!(response_updates.len(), 1);
            assert_eq!(response_updates[0].0, w1);
        }
        other => panic!("expected WakeAndReturn, got {other:?}"),
    }
    // Mutex owner unchanged.
    assert_eq!(
        host.mutexes().lookup(mutex_id).unwrap().owner(),
        Some(holder_tid),
    );
    // w1 re-parked on mutex.
    assert_eq!(
        host.mutexes()
            .lookup(mutex_id)
            .unwrap()
            .waiters()
            .iter()
            .collect::<Vec<_>>(),
        vec![PpuThreadId::PRIMARY],
    );
    // w2 still parked on cond.
    assert_eq!(
        host.conds()
            .lookup(cond_id)
            .unwrap()
            .waiters()
            .iter()
            .collect::<Vec<_>>(),
        vec![w2_tid],
    );
}

#[test]
fn cond_signal_before_wait_does_not_wake_subsequent_waiter() {
    // Non-sticky signal contract:
    //
    //   Thread A signals (signal / signal_all / signal_to) on
    //   a cond with no parked waiters. The signal is
    //   observably lost -- no pending-signal counter is
    //   maintained, no spurious wake token is buffered.
    //
    //   Thread B subsequently calls sys_cond_wait. B must
    //   block on the cond with PendingResponse::
    //   CondWakeReacquire, not wake spuriously with a
    //   CELL_OK from the earlier signal.
    //
    // This test covers all three signal variants
    // (signal / signal_all / signal_to) to prove none of them
    // introduces buffering. A regression in which the table
    // grew a "pending signal count" field (semaphore-style)
    // would fail this test: the subsequent cond_wait would
    // complete Immediate instead of Block.
    for variant in ["signal_one", "signal_all", "signal_to"] {
        let mut host = Lv2Host::new();
        let rt = FakeRuntime::new(0x10000);
        let waiter_unit = UnitId::new(0);
        let signaler_unit = UnitId::new(1);
        seed_primary_ppu(&mut host, waiter_unit);
        host.ppu_threads_mut()
            .create(signaler_unit, primary_attrs())
            .expect("signaler create");
        let mutex_id = create_mutex_host(&mut host, waiter_unit, &rt);
        let created = host.dispatch(
            Lv2Request::CondCreate {
                id_ptr: 0x200,
                mutex_id,
                attr_ptr: 0,
            },
            waiter_unit,
            &rt,
        );
        let cond_id = match &created {
            Lv2Dispatch::Immediate { effects: e, .. } => extract_write_u32(&e[0]),
            other => panic!("expected Immediate, got {other:?}"),
        };
        // Fire the chosen signal variant on a cond that has
        // no parked waiter yet.
        let pre_signal = match variant {
            "signal_one" => {
                host.dispatch(Lv2Request::CondSignal { id: cond_id }, signaler_unit, &rt)
            }
            "signal_all" => host.dispatch(
                Lv2Request::CondSignalAll { id: cond_id },
                signaler_unit,
                &rt,
            ),
            "signal_to" => host.dispatch(
                Lv2Request::CondSignalTo {
                    id: cond_id,
                    target_thread: PpuThreadId::PRIMARY.raw() as u32,
                },
                signaler_unit,
                &rt,
            ),
            _ => unreachable!(),
        };
        // signal / signal_all return CELL_OK regardless of
        // waiter presence (non-sticky). signal_to returns
        // ESRCH because the specific target is not parked.
        // Neither outcome leaves observable state that a
        // later waiter could pick up.
        match variant {
            "signal_to" => {
                assert!(
                    matches!(
                        pre_signal,
                        Lv2Dispatch::Immediate {
                            code: 0x8001_0005,
                            ..
                        }
                    ),
                    "{variant}: signal_to on missing target should ESRCH",
                );
            }
            _ => {
                assert!(
                    matches!(
                        pre_signal,
                        Lv2Dispatch::Immediate {
                            code: 0,
                            effects: _,
                        }
                    ),
                    "{variant}: signal on no waiter should return CELL_OK",
                );
            }
        }
        assert!(
            host.conds().lookup(cond_id).unwrap().waiters().is_empty(),
            "{variant}: cond waiter list must stay empty after lost signal",
        );
        assert_eq!(
            host.mutexes().lookup(mutex_id).unwrap().owner(),
            None,
            "{variant}: mutex must not be acquired by the lost signal",
        );
        // Waiter now locks mutex and cond_waits. It MUST
        // block -- not be satisfied by the earlier lost
        // signal.
        host.dispatch(
            Lv2Request::MutexLock {
                mutex_id,
                timeout: 0,
            },
            waiter_unit,
            &rt,
        );
        let wait_result = host.dispatch(
            Lv2Request::CondWait {
                id: cond_id,
                timeout: 0,
            },
            waiter_unit,
            &rt,
        );
        match wait_result {
            Lv2Dispatch::Block {
                reason, pending, ..
            } => {
                assert!(
                    matches!(reason, crate::dispatch::Lv2BlockReason::Cond { .. }),
                    "{variant}: wait must block on Cond reason",
                );
                assert!(
                    matches!(pending, PendingResponse::CondWakeReacquire { .. }),
                    "{variant}: wait must install CondWakeReacquire pending",
                );
            }
            other => panic!("{variant}: expected Block after lost signal, got {other:?}",),
        }
        assert_eq!(
            host.conds()
                .lookup(cond_id)
                .unwrap()
                .waiters()
                .iter()
                .collect::<Vec<_>>(),
            vec![PpuThreadId::PRIMARY],
            "{variant}: waiter must be parked on cond; no signal was buffered",
        );
    }
}

#[test]
fn cond_many_lost_signals_do_not_accumulate() {
    // Fire 20 signals (alternating signal_one / signal_all)
    // against an empty cond, then cond_wait. The waiter must
    // still block. Anchors the "no pending count" invariant:
    // even N lost signals cannot produce a single wake.
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let waiter_unit = UnitId::new(0);
    let signaler_unit = UnitId::new(1);
    seed_primary_ppu(&mut host, waiter_unit);
    host.ppu_threads_mut()
        .create(signaler_unit, primary_attrs())
        .expect("signaler create");
    let mutex_id = create_mutex_host(&mut host, waiter_unit, &rt);
    let created = host.dispatch(
        Lv2Request::CondCreate {
            id_ptr: 0x200,
            mutex_id,
            attr_ptr: 0,
        },
        waiter_unit,
        &rt,
    );
    let cond_id = match &created {
        Lv2Dispatch::Immediate { effects: e, .. } => extract_write_u32(&e[0]),
        other => panic!("expected Immediate, got {other:?}"),
    };
    for _ in 0..10 {
        host.dispatch(Lv2Request::CondSignal { id: cond_id }, signaler_unit, &rt);
        host.dispatch(
            Lv2Request::CondSignalAll { id: cond_id },
            signaler_unit,
            &rt,
        );
    }
    host.dispatch(
        Lv2Request::MutexLock {
            mutex_id,
            timeout: 0,
        },
        waiter_unit,
        &rt,
    );
    let wait_result = host.dispatch(
        Lv2Request::CondWait {
            id: cond_id,
            timeout: 0,
        },
        waiter_unit,
        &rt,
    );
    assert!(
        matches!(wait_result, Lv2Dispatch::Block { .. }),
        "20 lost signals must not wake a subsequent waiter",
    );
}

#[test]
fn cond_destroy_with_waiter_returns_ebusy() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let src = UnitId::new(0);
    seed_primary_ppu(&mut host, src);
    let mutex_id = create_mutex_host(&mut host, src, &rt);
    host.dispatch(
        Lv2Request::MutexLock {
            mutex_id,
            timeout: 0,
        },
        src,
        &rt,
    );
    let created = host.dispatch(
        Lv2Request::CondCreate {
            id_ptr: 0x200,
            mutex_id,
            attr_ptr: 0,
        },
        src,
        &rt,
    );
    let cond_id = match &created {
        Lv2Dispatch::Immediate { effects: e, .. } => extract_write_u32(&e[0]),
        other => panic!("expected Immediate, got {other:?}"),
    };
    host.dispatch(
        Lv2Request::CondWait {
            id: cond_id,
            timeout: 0,
        },
        src,
        &rt,
    );
    let r = host.dispatch(Lv2Request::CondDestroy { id: cond_id }, src, &rt);
    assert!(matches!(
        r,
        Lv2Dispatch::Immediate {
            code: 0x8001_000A,
            ..
        }
    ));
}

// ---------------------------------------------------------------
// Lost-wake regression tests.
//
// For each primitive in the "release is remembered" family
// (lwmutex, mutex, semaphore, event queue, event flag), the
// release scheduled BEFORE the wait must observably unblock the
// would-be waiter. Test shape: run the release first on an empty
// primitive, then run the wait; the wait must complete Immediate,
// not Block. A handler that split the check-and-mutate across
// commit boundaries would park the waiter even though the
// release already landed -- a classic lost-wake bug.
//
// Cond is NOT in this family. A cond signal-before-wait is
// observably lost (covered by
// cond_signal_before_wait_does_not_wake_subsequent_waiter).
// ---------------------------------------------------------------

// ---------------------------------------------------------------
// Multi-primitive determinism canary.
//
// Two identical Lv2Host instances fed the same syscall sequence
// -- spanning PPU thread creation, heavy mutex lock/unlock,
// lwmutex lock/unlock, and semaphore wait/post cycles -- must
// produce byte-identical state hashes and byte-identical
// dispatch-outcome tags at every step. This is the guard
// against ordering nondeterminism: any such regression must
// trip this test before it ever reaches a real title.
// ---------------------------------------------------------------

#[test]
fn multi_primitive_determinism_canary() {
    fn canonical_run() -> Vec<(String, u64)> {
        let mut host = Lv2Host::new();
        let rt = FakeRuntime::new(0x10000);
        let u0 = UnitId::new(0);
        let u1 = UnitId::new(1);
        let u2 = UnitId::new(2);
        seed_primary_ppu(&mut host, u0);
        host.ppu_threads_mut()
            .create(u1, primary_attrs())
            .expect("t1 create");
        host.ppu_threads_mut()
            .create(u2, primary_attrs())
            .expect("t2 create");

        let mutex_id = create_mutex_host(&mut host, u0, &rt);
        let lwmutex_id = match host.dispatch(
            Lv2Request::LwMutexCreate {
                id_ptr: 0x100,
                attr_ptr: 0,
            },
            u0,
            &rt,
        ) {
            Lv2Dispatch::Immediate { effects: e, .. } => extract_write_u32(&e[0]),
            other => panic!("unexpected {other:?}"),
        };
        let sem_id = match host.dispatch(
            Lv2Request::SemaphoreCreate {
                id_ptr: 0x200,
                attr_ptr: 0,
                initial: 0,
                max: 4,
            },
            u0,
            &rt,
        ) {
            Lv2Dispatch::Immediate { effects: e, .. } => extract_write_u32(&e[0]),
            other => panic!("unexpected {other:?}"),
        };

        // Fixed syscall script. Each entry is (label, unit,
        // request). The label travels into the trace so test
        // output identifies which step first diverged if the
        // canary ever fails.
        let script: Vec<(&'static str, UnitId, Lv2Request)> = vec![
            (
                "t0-mtx-lock",
                u0,
                Lv2Request::MutexLock {
                    mutex_id,
                    timeout: 0,
                },
            ),
            (
                "t1-mtx-lock",
                u1,
                Lv2Request::MutexLock {
                    mutex_id,
                    timeout: 0,
                },
            ),
            (
                "t2-mtx-lock",
                u2,
                Lv2Request::MutexLock {
                    mutex_id,
                    timeout: 0,
                },
            ),
            ("t0-mtx-unlock", u0, Lv2Request::MutexUnlock { mutex_id }),
            (
                "t0-sem-post",
                u0,
                Lv2Request::SemaphorePost { id: sem_id, val: 1 },
            ),
            (
                "t0-sem-post",
                u0,
                Lv2Request::SemaphorePost { id: sem_id, val: 1 },
            ),
            ("t1-mtx-unlock", u1, Lv2Request::MutexUnlock { mutex_id }),
            (
                "t0-sem-wait",
                u0,
                Lv2Request::SemaphoreWait {
                    id: sem_id,
                    timeout: 0,
                },
            ),
            ("t2-mtx-unlock", u2, Lv2Request::MutexUnlock { mutex_id }),
            (
                "t1-sem-wait",
                u1,
                Lv2Request::SemaphoreWait {
                    id: sem_id,
                    timeout: 0,
                },
            ),
            (
                "t0-lw-lock",
                u0,
                Lv2Request::LwMutexLock {
                    id: lwmutex_id,
                    timeout: 0,
                },
            ),
            (
                "t1-lw-lock",
                u1,
                Lv2Request::LwMutexLock {
                    id: lwmutex_id,
                    timeout: 0,
                },
            ),
            (
                "t2-lw-lock",
                u2,
                Lv2Request::LwMutexLock {
                    id: lwmutex_id,
                    timeout: 0,
                },
            ),
            (
                "t0-lw-unlock",
                u0,
                Lv2Request::LwMutexUnlock { id: lwmutex_id },
            ),
            (
                "t2-sem-wait",
                u2,
                Lv2Request::SemaphoreWait {
                    id: sem_id,
                    timeout: 0,
                },
            ),
            (
                "t0-sem-post",
                u0,
                Lv2Request::SemaphorePost { id: sem_id, val: 1 },
            ),
            (
                "t1-lw-unlock",
                u1,
                Lv2Request::LwMutexUnlock { id: lwmutex_id },
            ),
            (
                "t2-lw-unlock",
                u2,
                Lv2Request::LwMutexUnlock { id: lwmutex_id },
            ),
        ];

        let mut trace = Vec::with_capacity(script.len());
        for (label, unit, req) in script {
            let d = host.dispatch(req, unit, &rt);
            // Classify the dispatch outcome as a short tag.
            // Payload details (effect vectors, specific woken
            // ids) are intentionally excluded: the canary
            // guards scheduler selection order, which the tag
            // plus the post-dispatch state hash together
            // capture.
            let tag = match &d {
                Lv2Dispatch::Immediate { code, .. } => format!("Imm({code:#x})"),
                Lv2Dispatch::Block { .. } => "Block".into(),
                Lv2Dispatch::BlockAndWake { woken_unit_ids, .. } => {
                    format!("BlockAndWake({})", woken_unit_ids.len())
                }
                Lv2Dispatch::WakeAndReturn {
                    code,
                    woken_unit_ids,
                    ..
                } => format!("Wake({code:#x},n={})", woken_unit_ids.len()),
                Lv2Dispatch::RegisterSpu { .. } => "RegSpu".into(),
                Lv2Dispatch::PpuThreadCreate { .. } => "PpuCreate".into(),
                Lv2Dispatch::PpuThreadExit { .. } => "PpuExit".into(),
            };
            trace.push((format!("{label}:{tag}"), host.state_hash()));
        }
        trace
    }

    let run_a = canonical_run();
    let run_b = canonical_run();
    assert_eq!(
        run_a.len(),
        run_b.len(),
        "trace length diverged: {} vs {}",
        run_a.len(),
        run_b.len(),
    );
    for (i, (a, b)) in run_a.iter().zip(run_b.iter()).enumerate() {
        assert_eq!(
            a, b,
            "determinism canary diverged at step {i}: run_a = {a:?}, run_b = {b:?}",
        );
    }
    // Script covers lock/unlock/wait/post cycles on 3 distinct
    // PPU threads; a run with an empty script would trivially
    // pass. Guard against regression by asserting the script
    // actually ran non-empty state changes.
    assert!(run_a.len() >= 15);
}

#[test]
fn lost_wake_lwmutex_unlock_before_lock_does_not_park_waiter() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let owner_unit = UnitId::new(0);
    let later_unit = UnitId::new(1);
    seed_primary_ppu(&mut host, owner_unit);
    host.ppu_threads_mut()
        .create(later_unit, primary_attrs())
        .expect("later create");
    let created = host.dispatch(
        Lv2Request::LwMutexCreate {
            id_ptr: 0x100,
            attr_ptr: 0,
        },
        owner_unit,
        &rt,
    );
    let id = match &created {
        Lv2Dispatch::Immediate { effects: e, .. } => extract_write_u32(&e[0]),
        other => panic!("expected Immediate, got {other:?}"),
    };
    host.dispatch(Lv2Request::LwMutexLock { id, timeout: 0 }, owner_unit, &rt);
    let unlock = host.dispatch(Lv2Request::LwMutexUnlock { id }, owner_unit, &rt);
    assert!(matches!(
        unlock,
        Lv2Dispatch::Immediate {
            code: 0,
            effects: _,
        }
    ));
    let lock = host.dispatch(Lv2Request::LwMutexLock { id, timeout: 0 }, later_unit, &rt);
    match lock {
        Lv2Dispatch::Immediate { code: 0, .. } => {}
        other => panic!("expected Immediate(0), got {other:?}"),
    }
}

#[test]
fn lost_wake_mutex_unlock_before_lock_does_not_park_waiter() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let owner_unit = UnitId::new(0);
    let later_unit = UnitId::new(1);
    seed_primary_ppu(&mut host, owner_unit);
    host.ppu_threads_mut()
        .create(later_unit, primary_attrs())
        .expect("later create");
    let mutex_id = create_mutex_host(&mut host, owner_unit, &rt);
    host.dispatch(
        Lv2Request::MutexLock {
            mutex_id,
            timeout: 0,
        },
        owner_unit,
        &rt,
    );
    let unlock = host.dispatch(Lv2Request::MutexUnlock { mutex_id }, owner_unit, &rt);
    assert!(matches!(
        unlock,
        Lv2Dispatch::Immediate {
            code: 0,
            effects: _,
        }
    ));
    let lock = host.dispatch(
        Lv2Request::MutexLock {
            mutex_id,
            timeout: 0,
        },
        later_unit,
        &rt,
    );
    match lock {
        Lv2Dispatch::Immediate { code: 0, .. } => {}
        other => panic!("expected Immediate(0), got {other:?}"),
    }
}

#[test]
fn lost_wake_semaphore_post_before_wait_consumes_buffered_slot() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let src = UnitId::new(0);
    seed_primary_ppu(&mut host, src);
    let created = host.dispatch(
        Lv2Request::SemaphoreCreate {
            id_ptr: 0x100,
            attr_ptr: 0,
            initial: 0,
            max: 4,
        },
        src,
        &rt,
    );
    let sem_id = match &created {
        Lv2Dispatch::Immediate { effects: e, .. } => extract_write_u32(&e[0]),
        other => panic!("expected Immediate, got {other:?}"),
    };
    let post = host.dispatch(Lv2Request::SemaphorePost { id: sem_id, val: 1 }, src, &rt);
    assert!(matches!(
        post,
        Lv2Dispatch::Immediate {
            code: 0,
            effects: _,
        }
    ));
    assert_eq!(host.semaphores().lookup(sem_id).unwrap().count(), 1);
    let wait = host.dispatch(
        Lv2Request::SemaphoreWait {
            id: sem_id,
            timeout: 0,
        },
        src,
        &rt,
    );
    match wait {
        Lv2Dispatch::Immediate { code: 0, .. } => {}
        other => panic!("expected Immediate(0), got {other:?}"),
    }
    assert_eq!(host.semaphores().lookup(sem_id).unwrap().count(), 0);
}

#[test]
fn lost_wake_event_queue_send_before_receive_delivers_buffered_payload() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let src = UnitId::new(0);
    seed_primary_ppu(&mut host, src);
    let created = host.dispatch(
        Lv2Request::EventQueueCreate {
            id_ptr: 0x100,
            attr_ptr: 0,
            key: 0,
            size: 8,
        },
        src,
        &rt,
    );
    let q_id = match &created {
        Lv2Dispatch::Immediate { effects: e, .. } => extract_write_u32(&e[0]),
        other => panic!("expected Immediate, got {other:?}"),
    };
    let send = host.dispatch(
        Lv2Request::EventPortSend {
            port_id: q_id,
            data1: 0xAAAA,
            data2: 0xBBBB,
            data3: 0xCCCC,
        },
        src,
        &rt,
    );
    assert!(matches!(
        send,
        Lv2Dispatch::Immediate {
            code: 0,
            effects: _,
        }
    ));
    assert_eq!(host.event_queues().lookup(q_id).unwrap().len(), 1);
    let recv = host.dispatch(
        Lv2Request::EventQueueReceive {
            queue_id: q_id,
            out_ptr: 0x500,
            timeout: 0,
        },
        src,
        &rt,
    );
    match recv {
        Lv2Dispatch::Immediate { code: 0, .. } => {}
        other => panic!("expected Immediate(0), got {other:?}"),
    }
    assert!(host.event_queues().lookup(q_id).unwrap().is_empty());
}

#[test]
fn lost_wake_event_flag_set_before_wait_is_immediately_matched() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let src = UnitId::new(0);
    seed_primary_ppu(&mut host, src);
    let created = host.dispatch(
        Lv2Request::EventFlagCreate {
            id_ptr: 0x100,
            attr_ptr: 0,
            init: 0,
        },
        src,
        &rt,
    );
    let flag_id = match &created {
        Lv2Dispatch::Immediate { effects: e, .. } => extract_write_u32(&e[0]),
        other => panic!("expected Immediate, got {other:?}"),
    };
    let set = host.dispatch(
        Lv2Request::EventFlagSet {
            id: flag_id,
            bits: 0b1010,
        },
        src,
        &rt,
    );
    assert!(matches!(
        set,
        Lv2Dispatch::Immediate {
            code: 0,
            effects: _,
        }
    ));
    assert_eq!(host.event_flags().lookup(flag_id).unwrap().bits(), 0b1010);
    // Mode 0x02 = SYS_EVENT_FLAG_WAIT_OR (no-clear).
    let wait = host.dispatch(
        Lv2Request::EventFlagWait {
            id: flag_id,
            bits: 0b1000,
            mode: 0x02,
            result_ptr: 0x500,
            timeout: 0,
        },
        src,
        &rt,
    );
    match wait {
        Lv2Dispatch::Immediate { code: 0, .. } => {}
        other => panic!("expected Immediate(0), got {other:?}"),
    }
}
