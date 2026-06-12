//! SPU image import/open and thread-group dispatch tests: handle allocation, group lifecycle through RegisterSpu, and state-hash folding.

use super::*;
use crate::host::test_support::FakeRuntime;
use cellgov_mem::{GuestAddr, GuestMemory};
use cellgov_time::GuestTicks;

#[test]
fn image_import_registers_distinct_entries_per_type_id_img_ptr() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x1_0000);
    let req1 = Lv2Request::SpuImageImport {
        handle_out: 0x100,
        img_ptr: 0x200,
        size: 32,
        type_id: 0xAA,
    };
    let req2 = Lv2Request::SpuImageImport {
        handle_out: 0x200,
        img_ptr: 0x400,
        size: 32,
        type_id: 0xAA,
    };
    let r1 = host.dispatch(req1, UnitId::new(0), &rt);
    let r2 = host.dispatch(req2, UnitId::new(0), &rt);
    let (h1, h2) = match (&r1, &r2) {
        (
            Lv2Dispatch::Immediate {
                code: 0,
                effects: e1,
            },
            Lv2Dispatch::Immediate {
                code: 0,
                effects: e2,
            },
        ) => {
            let Effect::SharedWriteIntent { bytes: b1, .. } = &e1[0] else {
                panic!("e1");
            };
            let Effect::SharedWriteIntent { bytes: b2, .. } = &e2[0] else {
                panic!("e2");
            };
            (
                u32::from_be_bytes(b1.bytes()[..4].try_into().unwrap()),
                u32::from_be_bytes(b2.bytes()[..4].try_into().unwrap()),
            )
        }
        other => panic!("expected two Immediate code=0, got {other:?}"),
    };
    assert_ne!(h1, h2, "same type_id+img_ptr-distinct entries");
}

#[test]
fn image_import_out_of_range_img_ptr_returns_einval() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x1000);
    let req = Lv2Request::SpuImageImport {
        handle_out: 0x100,
        img_ptr: 0x800,
        size: 0x1000, // 0x800 + 0x1000 = 0x1800 > 0x1000
        type_id: 1,
    };
    let result = host.dispatch(req, UnitId::new(0), &rt);
    match result {
        Lv2Dispatch::Immediate { code, effects } => {
            assert_eq!(code, cell_errors::CELL_EINVAL.into());
            assert!(effects.is_empty());
        }
        other => panic!("expected Immediate, got {other:?}"),
    }
}

#[test]
fn image_import_unwritable_handle_out_returns_efault() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x1000);
    let req = Lv2Request::SpuImageImport {
        handle_out: 0xFF8, // 0xFF8 + 16 = 0x1008 > 0x1000
        img_ptr: 0x100,
        size: 32,
        type_id: 1,
    };
    let result = host.dispatch(req, UnitId::new(0), &rt);
    match result {
        Lv2Dispatch::Immediate { code, effects } => {
            assert_eq!(code, cell_errors::CELL_EFAULT.into());
            assert!(effects.is_empty());
        }
        other => panic!("expected Immediate, got {other:?}"),
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
    if let Lv2Dispatch::Immediate { effects, .. } = r1 {
        assert_eq!(
            effects[0].clone(),
            Effect::SharedWriteIntent {
                range: ByteRange::new(GuestAddr::new(0x100), 4).unwrap(),
                bytes: WritePayload::from_slice(&1u32.to_be_bytes()),
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
                bytes: WritePayload::from_slice(&2u32.to_be_bytes()),
                ordering: PriorityClass::Normal,
                source: UnitId::new(0),
                source_time: GuestTicks::ZERO,
            }
        );
    }
}

#[test]
fn group_create_rejects_oversized_num_threads() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x4000);
    let req = Lv2Request::SpuThreadGroupCreate {
        id_ptr: 0x100,
        num_threads: 300,
        priority: 0,
        attr_ptr: 0,
    };
    let result = host.dispatch(req, UnitId::new(0), &rt);
    match result {
        Lv2Dispatch::Immediate { code, effects } => {
            assert_eq!(code, cell_errors::CELL_EINVAL.into());
            assert!(effects.is_empty());
        }
        other => panic!("expected Immediate, got {other:?}"),
    }
    assert_eq!(host.thread_groups().len(), 0);
}

#[test]
fn thread_initialize_records_slot() {
    let mut host = Lv2Host::new();
    host.content_store_mut().register(b"/spu.elf", vec![0xAA]);

    // img_ptr at 0x200: handle=1 pre-populated (as image_open would write).
    let mut mem = GuestMemory::new(0x4000);
    let img_range = ByteRange::new(GuestAddr::new(0x200), 4).unwrap();
    mem.apply_commit(img_range, &1u32.to_be_bytes()).unwrap();
    let rt = FakeRuntime::with_memory(mem);

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
            assert_eq!(effects.len(), 1);
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

    let mut mem = GuestMemory::new(0x4000);
    let path = b"/spu.elf\0";
    let path_range = ByteRange::new(GuestAddr::new(0x100), path.len() as u64).unwrap();
    mem.apply_commit(path_range, path).unwrap();
    let img_range = ByteRange::new(GuestAddr::new(0x300), 4).unwrap();
    mem.apply_commit(img_range, &1u32.to_be_bytes()).unwrap();

    // sys_spu_thread_argument: 4 x u64 big-endian; arg0 = 0x1000.
    let mut arg_bytes = [0u8; 32];
    arg_bytes[0..8].copy_from_slice(&0x1000u64.to_be_bytes());
    let arg_range = ByteRange::new(GuestAddr::new(0x200), 32).unwrap();
    mem.apply_commit(arg_range, &arg_bytes).unwrap();

    let rt = FakeRuntime::with_memory(mem);

    host.dispatch(
        Lv2Request::SpuImageOpen {
            img_ptr: 0x300,
            path_ptr: 0x100,
        },
        UnitId::new(0),
        &rt,
    );

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

    let result = host.dispatch(
        Lv2Request::SpuThreadGroupStart { group_id: 1 },
        UnitId::new(0),
        &rt,
    );

    match result {
        Lv2Dispatch::RegisterSpu { inits, code, .. } => {
            assert_eq!(code, 0);
            assert_eq!(inits.len(), 1);
            let init = inits.get(&0).expect("slot 0 init");
            assert_eq!(init.ls_bytes, vec![0xAA, 0xBB]);
            assert_eq!(init.entry_pc, 0x80);
            assert_eq!(init.stack_ptr, 0x3FFF0);
            assert_eq!(init.args[0], 0x1000);
            assert_eq!(init.group_id, 1);
            assert!(inits.contains_key(&0));
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
