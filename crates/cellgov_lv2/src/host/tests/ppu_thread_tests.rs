//! PPU-thread lifecycle dispatch tests: create with stack allocation and r13 seeding, exit waking join waiters, join blocking, and yield.

use super::*;
use crate::host::test_support::{opd_runtime, opd_runtime_with_tls, primary_attrs, FakeRuntime};
use crate::request::Lv2Request;

#[test]
fn ppu_thread_exit_marks_thread_finished_with_exit_value() {
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
            ..
        } => {
            assert_eq!(exit_value, 0xDEAD_BEEF);
            assert!(woken_unit_ids.is_empty());
            assert!(effects.is_empty());
        }
        other => panic!("expected PpuThreadExit dispatch, got {other:?}"),
    }
    let primary = host.ppu_thread_for_unit(UnitId::new(0)).unwrap();
    assert_eq!(primary.state, crate::ppu_thread::PpuThreadState::Finished);
    assert_eq!(primary.exit_value, Some(0xDEAD_BEEF));
}

#[test]
fn ppu_thread_exit_unseeded_thread_still_returns_dispatch() {
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
    let mut host = Lv2Host::new();
    host.seed_primary_ppu_thread(UnitId::new(0), primary_attrs());
    let child_tid = host
        .ppu_threads_mut()
        .create(UnitId::new(1), primary_attrs())
        .expect("child create");
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
            assert_eq!(code, cell_errors::CELL_ESRCH.into());
            assert!(effects.is_empty());
        }
        other => panic!("expected Immediate with ESRCH, got {other:?}"),
    }
}

#[test]
fn ppu_thread_join_detached_target_returns_einval() {
    // RPCS3 sys_ppu_thread.cpp sys_ppu_thread_join: a detached
    // target is EINVAL; ESRCH is reserved for unknown ids.
    let mut host = Lv2Host::new();
    host.seed_primary_ppu_thread(UnitId::new(0), primary_attrs());
    let child = host
        .ppu_threads_mut()
        .create(UnitId::new(1), primary_attrs())
        .expect("child create");
    assert!(host.ppu_threads_mut().detach(child));
    let rt = FakeRuntime::new(256);
    let result = host.dispatch(
        Lv2Request::PpuThreadJoin {
            target: child.raw(),
            status_out_ptr: 0x500,
        },
        UnitId::new(0),
        &rt,
    );
    assert_eq!(
        result,
        Lv2Dispatch::immediate(cell_errors::CELL_EINVAL.into())
    );
}

#[test]
fn ppu_thread_join_finished_target_with_null_status_ptr_is_efault_without_write() {
    // RPCS3 sys_ppu_thread.cpp sys_ppu_thread_join checks vptr after
    // the join concludes and returns EFAULT instead of storing.
    let mut host = Lv2Host::new();
    host.seed_primary_ppu_thread(UnitId::new(0), primary_attrs());
    let child = host
        .ppu_threads_mut()
        .create(UnitId::new(1), primary_attrs())
        .expect("child create");
    host.ppu_threads_mut().mark_finished(child, 0xFEED_FACE);
    let rt = FakeRuntime::new(0x10000);
    let result = host.dispatch(
        Lv2Request::PpuThreadJoin {
            target: child.raw(),
            status_out_ptr: 0,
        },
        UnitId::new(0),
        &rt,
    );
    assert_eq!(
        result,
        Lv2Dispatch::immediate(cell_errors::CELL_EFAULT.into())
    );
}

#[test]
fn ppu_thread_create_returns_dispatch_with_allocated_stack() {
    let mut host = Lv2Host::new();
    let rt = opd_runtime(0x200, 0x10_0000, 0x10_0100);
    let result = host.dispatch(
        Lv2Request::PpuThreadCreate {
            id_ptr: 0x1000,
            param_ptr: 0x200,
            arg: 0xDEAD_BEEF,
            unk: 0,
            priority: 1500,
            stacksize: 0x10_000,
            flags: 0,
            threadname_ptr: 0,
        },
        UnitId::new(0),
        &rt,
    );
    match result {
        Lv2Dispatch::PpuThreadCreate {
            id_ptr,
            init,
            stack_base,
            stack_size,
            priority,
            effects,
        } => {
            assert_eq!(id_ptr, 0x1000);
            assert_eq!(init.entry_code, 0x10_0000);
            assert_eq!(init.entry_toc, 0x10_0100);
            assert_eq!(init.arg, 0xDEAD_BEEF);
            assert_eq!(priority, 1500);
            assert_eq!(stack_base, 0xD010_0000);
            assert_eq!(stack_size, 0x10_000);
            assert_eq!(init.stack_top, 0xD011_0000 - 0x10);
            assert!(effects.is_empty());
        }
        other => panic!("expected PpuThreadCreate, got {other:?}"),
    }
}

#[test]
fn ppu_thread_create_seeds_r13_from_param_tls_verbatim() {
    let mut host = Lv2Host::new();
    let rt = opd_runtime_with_tls(0x200, 0x10_0000, 0x10_0100, 0x028e_7220);
    let result = host.dispatch(
        Lv2Request::PpuThreadCreate {
            id_ptr: 0x1000,
            param_ptr: 0x200,
            arg: 0,
            unk: 0,
            priority: 1000,
            stacksize: 0x8000,
            flags: 0,
            threadname_ptr: 0,
        },
        UnitId::new(0),
        &rt,
    );
    match result {
        Lv2Dispatch::PpuThreadCreate { init, .. } => {
            assert_eq!(init.tls_base, 0x028e_7220);
        }
        other => panic!("expected PpuThreadCreate, got {other:?}"),
    }
}

#[test]
fn ppu_thread_create_passes_zero_param_tls_through_unvalidated() {
    let mut host = Lv2Host::new();
    let rt = opd_runtime(0x200, 0, 0);
    let result = host.dispatch(
        Lv2Request::PpuThreadCreate {
            id_ptr: 0x1000,
            param_ptr: 0x200,
            arg: 0,
            unk: 0,
            priority: 1000,
            stacksize: 0x8000,
            flags: 0,
            threadname_ptr: 0,
        },
        UnitId::new(0),
        &rt,
    );
    match result {
        Lv2Dispatch::PpuThreadCreate { init, .. } => {
            assert_eq!(init.tls_base, 0);
        }
        other => panic!("expected PpuThreadCreate, got {other:?}"),
    }
}

#[test]
fn ppu_thread_create_enforces_minimum_stack_size() {
    let mut host = Lv2Host::new();
    let rt = opd_runtime(0x200, 0, 0);
    let result = host.dispatch(
        Lv2Request::PpuThreadCreate {
            id_ptr: 0x1000,
            param_ptr: 0x200,
            arg: 0,
            unk: 0,
            priority: 1000,
            stacksize: 0x100,
            flags: 0,
            threadname_ptr: 0,
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
fn ppu_thread_create_bad_opd_returns_efault() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x100);
    let result = host.dispatch(
        Lv2Request::PpuThreadCreate {
            id_ptr: 0x10,
            param_ptr: 0xDEAD_BEEF,
            arg: 0,
            unk: 0,
            priority: 1000,
            stacksize: 0x4000,
            flags: 0,
            threadname_ptr: 0,
        },
        UnitId::new(0),
        &rt,
    );
    assert_eq!(
        result,
        Lv2Dispatch::immediate(cell_errors::CELL_EFAULT.into())
    );
}

#[test]
fn ppu_thread_create_bad_opd_via_param_returns_efault() {
    let mut mem = cellgov_mem::GuestMemory::new(0x1_0000);
    let mut param_bytes = [0u8; 8];
    param_bytes[0..4].copy_from_slice(&0xDEAD_BEEFu32.to_be_bytes());
    let param_range = cellgov_mem::ByteRange::new(cellgov_mem::GuestAddr::new(0x200), 8).unwrap();
    mem.apply_commit(param_range, &param_bytes).unwrap();
    let rt = crate::host::test_support::FakeRuntime::with_memory(mem);

    let mut host = Lv2Host::new();
    let result = host.dispatch(
        Lv2Request::PpuThreadCreate {
            id_ptr: 0x10,
            param_ptr: 0x200,
            arg: 0,
            unk: 0,
            priority: 1000,
            stacksize: 0x4000,
            flags: 0,
            threadname_ptr: 0,
        },
        UnitId::new(0),
        &rt,
    );
    assert_eq!(
        result,
        Lv2Dispatch::immediate(cell_errors::CELL_EFAULT.into())
    );
}

#[test]
fn ppu_thread_create_null_entry_descriptor_is_efault() {
    // param.entry_opd_ptr == 0: RPCS3 sys_ppu_thread.cpp
    // _sys_ppu_thread_create rejects !param->entry with EFAULT.
    // Without the check, the zeroed arena at address 0 silently
    // supplies an all-zero OPD.
    let mut mem = cellgov_mem::GuestMemory::new(0x1_0000);
    let param_range = cellgov_mem::ByteRange::new(cellgov_mem::GuestAddr::new(0x200), 8).unwrap();
    mem.apply_commit(param_range, &[0u8; 8]).unwrap();
    let rt = FakeRuntime::with_memory(mem);
    let mut host = Lv2Host::new();
    let result = host.dispatch(
        Lv2Request::PpuThreadCreate {
            id_ptr: 0x10,
            param_ptr: 0x200,
            arg: 0,
            unk: 0,
            priority: 1000,
            stacksize: 0x4000,
            flags: 0,
            threadname_ptr: 0,
        },
        UnitId::new(0),
        &rt,
    );
    assert_eq!(
        result,
        Lv2Dispatch::immediate(cell_errors::CELL_EFAULT.into())
    );
}

#[test]
fn ppu_thread_create_priority_above_3071_is_einval() {
    // RPCS3 sys_ppu_thread.cpp _sys_ppu_thread_create: prio > 3071
    // is EINVAL regardless of capability.
    let mut host = Lv2Host::new();
    let rt = opd_runtime(0x200, 0x10_0000, 0x10_0100);
    let result = host.dispatch(
        Lv2Request::PpuThreadCreate {
            id_ptr: 0x1000,
            param_ptr: 0x200,
            arg: 0,
            unk: 0,
            priority: 3072,
            stacksize: 0x8000,
            flags: 0,
            threadname_ptr: 0,
        },
        UnitId::new(0),
        &rt,
    );
    assert_eq!(
        result,
        Lv2Dispatch::immediate(cell_errors::CELL_EINVAL.into())
    );
}

#[test]
fn ppu_thread_create_negative_priority_is_einval_for_user_perm() {
    // RPCS3 sys_ppu_thread.cpp _sys_ppu_thread_create: the floor is
    // 0 without debug-or-root capability.
    let mut host = Lv2Host::new();
    let rt = opd_runtime(0x200, 0x10_0000, 0x10_0100);
    let result = host.dispatch(
        Lv2Request::PpuThreadCreate {
            id_ptr: 0x1000,
            param_ptr: 0x200,
            arg: 0,
            unk: 0,
            priority: -1,
            stacksize: 0x8000,
            flags: 0,
            threadname_ptr: 0,
        },
        UnitId::new(0),
        &rt,
    );
    assert_eq!(
        result,
        Lv2Dispatch::immediate(cell_errors::CELL_EINVAL.into())
    );
}

#[test]
fn ppu_thread_create_negative_priority_floor_drops_to_minus_512_with_debug_or_root() {
    // RPCS3 sys_ppu_thread.cpp _sys_ppu_thread_create: debug-or-root
    // callers may go down to -512; -513 stays EINVAL.
    let mut host = Lv2Host::new();
    host.set_control_flags1(0x4000_0000); // root -> debug_or_root
    let rt = opd_runtime(0x200, 0x10_0000, 0x10_0100);
    let create = |host: &mut Lv2Host, priority: i32| {
        host.dispatch(
            Lv2Request::PpuThreadCreate {
                id_ptr: 0x1000,
                param_ptr: 0x200,
                arg: 0,
                unk: 0,
                priority,
                stacksize: 0x8000,
                flags: 0,
                threadname_ptr: 0,
            },
            UnitId::new(0),
            &rt,
        )
    };
    assert!(matches!(
        create(&mut host, -512),
        Lv2Dispatch::PpuThreadCreate { .. }
    ));
    assert_eq!(
        create(&mut host, -513),
        Lv2Dispatch::immediate(cell_errors::CELL_EINVAL.into())
    );
}

#[test]
fn ppu_thread_yield_returns_ok_with_no_effects() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(256);
    let result = host.dispatch(Lv2Request::PpuThreadYield, UnitId::new(0), &rt);
    assert_eq!(result, Lv2Dispatch::immediate(0));
}
