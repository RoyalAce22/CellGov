//! SPU image import/open and thread-group dispatch tests: handle allocation, group lifecycle through RegisterSpu, and sync-partial folding.

use super::*;
use crate::host::test_support::FakeRuntime;
use cellgov_mem::{GuestAddr, GuestMemory};
use cellgov_ps3_abi::format::elf::{ELF32_E_ENTRY, ELF32_HEADER_SIZE};
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
            // Kernel-shaped record: type word first, the id in entry_point.
            assert_eq!(u32::from_be_bytes(b1.bytes()[..4].try_into().unwrap()), 1);
            assert_eq!(u32::from_be_bytes(b2.bytes()[..4].try_into().unwrap()), 1);
            (
                u32::from_be_bytes(b1.bytes()[4..8].try_into().unwrap()),
                u32::from_be_bytes(b2.bytes()[4..8].try_into().unwrap()),
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
            assert_eq!(code, errno::CELL_EINVAL.into());
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
            assert_eq!(code, errno::CELL_EFAULT.into());
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
    match &r1 {
        Lv2Dispatch::Immediate { effects, .. } => {
            assert_eq!(
                effects[0],
                Effect::shared_write(
                    ByteRange::new(GuestAddr::new(0x100), 4).unwrap(),
                    WritePayload::from_slice(&1u32.to_be_bytes()),
                    UnitId::new(0),
                    GuestTicks::ZERO
                )
            );
        }
        other => panic!("expected Immediate, got {other:?}"),
    }
    match &r2 {
        Lv2Dispatch::Immediate { effects, .. } => {
            assert_eq!(
                effects[0],
                Effect::shared_write(
                    ByteRange::new(GuestAddr::new(0x200), 4).unwrap(),
                    WritePayload::from_slice(&2u32.to_be_bytes()),
                    UnitId::new(0),
                    GuestTicks::ZERO
                )
            );
        }
        other => panic!("expected Immediate, got {other:?}"),
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
            assert_eq!(code, errno::CELL_EINVAL.into());
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

    // img_ptr at 0x200: the kernel-shaped record image_open writes,
    // type KERNEL with image id 1 in entry_point.
    let mut mem = GuestMemory::new(0x4000);
    let img_range = ByteRange::new(GuestAddr::new(0x200), 8).unwrap();
    mem.apply_commit(img_range, &[0, 0, 0, 1, 0, 0, 0, 1])
        .unwrap();
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
fn sync_partial_changes_when_image_registered() {
    let empty = Lv2Host::new();
    let mut populated = Lv2Host::new();
    populated.content_store_mut().register(b"/spu.elf", vec![]);
    assert_ne!(empty.sync_partial(), populated.sync_partial());
}

#[test]
fn sync_partial_deterministic_across_instances() {
    let mut a = Lv2Host::new();
    let mut b = Lv2Host::new();
    a.content_store_mut().register(b"/spu.elf", vec![1, 2]);
    b.content_store_mut().register(b"/spu.elf", vec![1, 2]);
    assert_eq!(a.sync_partial(), b.sync_partial());
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
    // Kernel-shaped record: type KERNEL, image id 1 in entry_point.
    let img_range = ByteRange::new(GuestAddr::new(0x300), 8).unwrap();
    mem.apply_commit(img_range, &[0, 0, 0, 1, 0, 0, 0, 1])
        .unwrap();

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
            assert_eq!(
                init.image,
                crate::dispatch::SpuLoadImage::Elf(vec![0xAA, 0xBB])
            );
            assert_eq!(init.entry_pc, 0, "a header too short to hold e_entry");
            assert_eq!(init.stack_ptr, 0x3FFF0);
            assert_eq!(init.args[0], 0x1000);
            assert_eq!(init.group_id, 1);
            assert!(inits.contains_key(&0));
        }
        other => panic!("expected RegisterSpu, got {other:?}"),
    }
}

/// Drive open -> create -> initialize -> start for a path-registered
/// image and report slot 0's entry pc.
fn kernel_image_entry_pc(elf: Vec<u8>) -> u32 {
    let mut host = Lv2Host::new();
    host.content_store_mut().register(b"/spu.elf", elf);

    let mut mem = GuestMemory::new(0x4000);
    let path = b"/spu.elf\0";
    let path_range = ByteRange::new(GuestAddr::new(0x100), path.len() as u64).unwrap();
    mem.apply_commit(path_range, path).unwrap();
    let img_range = ByteRange::new(GuestAddr::new(0x300), 8).unwrap();
    mem.apply_commit(img_range, &[0, 0, 0, 1, 0, 0, 0, 1])
        .unwrap();
    let arg_range = ByteRange::new(GuestAddr::new(0x200), 32).unwrap();
    mem.apply_commit(arg_range, &[0u8; 32]).unwrap();
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
    let Lv2Dispatch::RegisterSpu { inits, .. } = result else {
        panic!("expected RegisterSpu");
    };
    inits[&0].entry_pc
}

#[test]
fn a_kernel_image_enters_at_its_own_elf_entry() {
    let mut elf = vec![0u8; ELF32_HEADER_SIZE];
    elf[ELF32_E_ENTRY..ELF32_E_ENTRY + 4].copy_from_slice(&0x2340u32.to_be_bytes());
    assert_eq!(kernel_image_entry_pc(elf), 0x2340);
}

#[test]
fn the_shortest_prefix_holding_e_entry_still_reports_it() {
    let mut elf = vec![0u8; ELF32_E_ENTRY + 4];
    elf[ELF32_E_ENTRY..].copy_from_slice(&0x1234_5678u32.to_be_bytes());
    assert_eq!(kernel_image_entry_pc(elf.clone()), 0x1234_5678);
    elf.truncate(ELF32_E_ENTRY + 3);
    assert_eq!(kernel_image_entry_pc(elf), 0);
}

/// Group 1 created, driven to Running, and its single SPU finished.
fn host_with_finished_group() -> Lv2Host {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x4000);
    host.dispatch(
        Lv2Request::SpuThreadGroupCreate {
            id_ptr: 0x100,
            num_threads: 1,
            priority: 0,
            attr_ptr: 0,
        },
        UnitId::new(0),
        &rt,
    );
    host.thread_groups_mut().get_mut(1).unwrap().state = GroupState::Running;
    host.record_spu(UnitId::new(7), 1, 0).unwrap();
    assert_eq!(host.notify_spu_finished(UnitId::new(7)), Ok(Some(1)));
    host
}

fn join_finished_group(cause_ptr: u32, status_ptr: u32) -> Lv2Dispatch {
    let mut host = host_with_finished_group();
    let rt = FakeRuntime::new(0x4000);
    host.dispatch(
        Lv2Request::SpuThreadGroupJoin {
            group_id: 1,
            cause_ptr,
            status_ptr,
        },
        UnitId::new(0),
        &rt,
    )
}

#[test]
fn a_finished_group_join_with_null_cause_writes_nothing_and_returns_efault() {
    match join_finished_group(0, 0x300) {
        Lv2Dispatch::Immediate { code, effects } => {
            assert_eq!(code, errno::CELL_EFAULT.into());
            assert!(effects.is_empty());
        }
        other => panic!("expected Immediate, got {other:?}"),
    }
}

#[test]
fn a_finished_group_join_with_both_pointers_null_writes_nothing_and_returns_efault() {
    match join_finished_group(0, 0) {
        Lv2Dispatch::Immediate { code, effects } => {
            assert_eq!(code, errno::CELL_EFAULT.into());
            assert!(effects.is_empty());
        }
        other => panic!("expected Immediate, got {other:?}"),
    }
}

#[test]
fn a_finished_group_join_with_null_status_writes_cause_only_and_returns_efault() {
    match join_finished_group(0x300, 0) {
        Lv2Dispatch::Immediate { code, effects } => {
            assert_eq!(code, errno::CELL_EFAULT.into());
            assert_eq!(effects.len(), 1);
            let Effect::SharedWriteIntent { range, bytes, .. } = &effects[0] else {
                panic!("expected SharedWriteIntent, got {:?}", effects[0]);
            };
            assert_eq!(range.start().raw(), 0x300);
            assert_eq!(range.length(), 4);
            assert_eq!(
                bytes.bytes(),
                &spu::group_join_cause::GROUP_EXIT.to_be_bytes()
            );
        }
        other => panic!("expected Immediate, got {other:?}"),
    }
}

#[test]
fn a_finished_group_join_with_both_pointers_writes_both_and_returns_ok() {
    match join_finished_group(0x300, 0x400) {
        Lv2Dispatch::Immediate { code, effects } => {
            assert_eq!(code, 0);
            assert_eq!(effects.len(), 2);
            let Effect::SharedWriteIntent { range, bytes, .. } = &effects[0] else {
                panic!("expected SharedWriteIntent, got {:?}", effects[0]);
            };
            assert_eq!(range.start().raw(), 0x300);
            assert_eq!(
                bytes.bytes(),
                &spu::group_join_cause::GROUP_EXIT.to_be_bytes()
            );
            let Effect::SharedWriteIntent { range, bytes, .. } = &effects[1] else {
                panic!("expected SharedWriteIntent, got {:?}", effects[1]);
            };
            assert_eq!(range.start().raw(), 0x400);
            assert_eq!(bytes.bytes(), &0u32.to_be_bytes());
        }
        other => panic!("expected Immediate, got {other:?}"),
    }
}

#[test]
fn an_out_of_range_slot_outranks_an_unreadable_image_pointer() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x100);
    let result = host.dispatch(
        Lv2Request::SpuThreadInitialize {
            thread_ptr: 0x10,
            group_id: 1,
            thread_num: MAX_SLOTS_PER_GROUP,
            img_ptr: 0xDEAD_0000,
            attr_ptr: 0,
            arg_ptr: 0,
        },
        UnitId::new(0),
        &rt,
    );
    match result {
        Lv2Dispatch::Immediate { code, .. } => {
            assert_eq!(code, errno::CELL_EINVAL.into());
        }
        other => panic!("expected Immediate EINVAL, got {other:?}"),
    }
}

#[test]
fn a_zero_thread_group_create_is_rejected_as_einval() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x1000);
    let result = host.dispatch(
        Lv2Request::SpuThreadGroupCreate {
            id_ptr: 0x100,
            num_threads: 0,
            priority: 0,
            attr_ptr: 0,
        },
        UnitId::new(0),
        &rt,
    );
    match result {
        Lv2Dispatch::Immediate { code, effects } => {
            assert_eq!(code, errno::CELL_EINVAL.into());
            assert!(effects.is_empty(), "a refused create writes no group id");
        }
        other => panic!("expected Immediate, got {other:?}"),
    }
    assert_eq!(host.thread_groups().len(), 0, "no group may be allocated");
}

#[test]
fn a_second_start_of_a_running_group_is_rejected_as_estat() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x4000);
    host.dispatch(
        Lv2Request::SpuThreadGroupCreate {
            id_ptr: 0x100,
            num_threads: 1,
            priority: 0,
            attr_ptr: 0,
        },
        UnitId::new(0),
        &rt,
    );
    host.thread_groups_mut().get_mut(1).unwrap().state = GroupState::Running;
    let result = host.dispatch(
        Lv2Request::SpuThreadGroupStart { group_id: 1 },
        UnitId::new(0),
        &rt,
    );
    match result {
        Lv2Dispatch::Immediate { code, .. } => {
            assert_eq!(code, errno::CELL_ESTAT.into());
        }
        other => panic!("expected Immediate ESTAT, got {other:?}"),
    }
}

#[test]
fn a_start_of_a_finished_group_is_rejected_as_estat() {
    let mut host = host_with_finished_group();
    let rt = FakeRuntime::new(0x4000);
    let result = host.dispatch(
        Lv2Request::SpuThreadGroupStart { group_id: 1 },
        UnitId::new(0),
        &rt,
    );
    match result {
        Lv2Dispatch::Immediate { code, .. } => {
            assert_eq!(code, errno::CELL_ESTAT.into());
        }
        other => panic!("expected Immediate ESTAT, got {other:?}"),
    }
}

#[test]
fn initializing_a_thread_in_a_started_group_is_rejected_as_ebusy() {
    let mut host = Lv2Host::new();
    let handle = host.content_store_mut().register(b"/spu.elf", vec![0xAA]);

    let mut mem = GuestMemory::new(0x4000);
    let mut record = [0u8; 8];
    record[3] = 1;
    record[4..8].copy_from_slice(&handle.raw().to_be_bytes());
    let img_range = ByteRange::new(GuestAddr::new(0x300), 8).unwrap();
    mem.apply_commit(img_range, &record).unwrap();
    let rt = FakeRuntime::with_memory(mem);

    host.dispatch(
        Lv2Request::SpuThreadGroupCreate {
            id_ptr: 0x400,
            num_threads: 2,
            priority: 0,
            attr_ptr: 0,
        },
        UnitId::new(0),
        &rt,
    );
    host.thread_groups_mut().get_mut(1).unwrap().state = GroupState::Running;

    let result = host.dispatch(
        Lv2Request::SpuThreadInitialize {
            thread_ptr: 0x500,
            group_id: 1,
            thread_num: 1,
            img_ptr: 0x300,
            attr_ptr: 0,
            arg_ptr: 0,
        },
        UnitId::new(0),
        &rt,
    );
    match result {
        Lv2Dispatch::Immediate { code, effects } => {
            assert_eq!(code, errno::CELL_EBUSY.into());
            assert!(effects.is_empty(), "a refused initialize writes no id");
        }
        other => panic!("expected Immediate EBUSY, got {other:?}"),
    }
}

#[test]
fn a_slot_index_past_the_declared_count_is_accepted_and_a_full_group_is_ebusy() {
    let mut host = Lv2Host::new();
    let handle = host.content_store_mut().register(b"/spu.elf", vec![0xAA]);

    let mut mem = GuestMemory::new(0x4000);
    let mut record = [0u8; 8];
    record[3] = 1;
    record[4..8].copy_from_slice(&handle.raw().to_be_bytes());
    let img_range = ByteRange::new(GuestAddr::new(0x300), 8).unwrap();
    mem.apply_commit(img_range, &record).unwrap();
    let rt = FakeRuntime::with_memory(mem);

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

    let result = host.dispatch(
        Lv2Request::SpuThreadInitialize {
            thread_ptr: 0x500,
            group_id: 1,
            thread_num: 1,
            img_ptr: 0x300,
            attr_ptr: 0,
            arg_ptr: 0,
        },
        UnitId::new(0),
        &rt,
    );
    match result {
        Lv2Dispatch::Immediate { code, .. } => {
            assert_eq!(code, 0, "slot 1 of a one-thread group is a legal index");
        }
        other => panic!("expected Immediate, got {other:?}"),
    }
    let result = host.dispatch(
        Lv2Request::SpuThreadInitialize {
            thread_ptr: 0x500,
            group_id: 1,
            thread_num: 0,
            img_ptr: 0x300,
            attr_ptr: 0,
            arg_ptr: 0,
        },
        UnitId::new(0),
        &rt,
    );
    match result {
        Lv2Dispatch::Immediate { code, effects } => {
            assert_eq!(code, errno::CELL_EBUSY.into());
            assert!(effects.is_empty());
        }
        other => panic!("expected Immediate EBUSY, got {other:?}"),
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
