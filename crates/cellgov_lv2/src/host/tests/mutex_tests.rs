//! LV2 mutex dispatch tests: attribute decoding, contended block-and-wake, recursive relock counting, trylock EBUSY, EPERM non-owner unlock, and transient-unit aliasing.

use super::*;
use crate::host::test_support::{extract_write_u32, seed_primary_ppu, FakeRuntime};
use crate::ppu_thread::{PpuThreadAttrs, PpuThreadId};
use crate::request::Lv2Request;

#[test]
fn mutex_create_allocates_monotonic_ids() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let source = UnitId::new(0);

    let r1 = host.dispatch(
        Lv2Request::MutexCreate {
            id_ptr: 0x100,
            attr_ptr: 0,
        },
        source,
        &rt,
    );
    let r2 = host.dispatch(
        Lv2Request::MutexCreate {
            id_ptr: 0x104,
            attr_ptr: 0,
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
    let Lv2Dispatch::Immediate { code, .. } = r else {
        panic!("expected Immediate, got {r:?}");
    };
    assert_eq!(code, cell_errors::CELL_ESRCH.into());
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
fn mutex_create_default_attrs_when_attr_ptr_zero() {
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
    let mut mem = cellgov_mem::GuestMemory::new(0x10000);
    let attr_bytes = [
        0x00, 0x00, 0x00, 0x02, // protocol = SYS_SYNC_PRIORITY
        0x00, 0x00, 0x00, 0x10, // recursive = SYS_SYNC_RECURSIVE
        0x00, 0x00, 0x02, 0x00, // pshared = SYS_SYNC_NOT_PROCESS_SHARED
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
    assert_eq!(attrs.protocol, 0x02);
    assert_eq!(attrs.priority_policy, 0x02);
    assert!(attrs.recursive);
}

#[test]
fn a_not_recursive_attr_creates_a_non_recursive_mutex() {
    // SYS_SYNC_NOT_RECURSIVE (0x20) is nonzero but means NOT
    // recursive; only SYS_SYNC_RECURSIVE (0x10) enables re-locking
    // (RPCS3 sys_mutex.cpp sys_mutex_create).
    let mut mem = cellgov_mem::GuestMemory::new(0x10000);
    let attr_bytes = [
        0x00, 0x00, 0x00, 0x01, // protocol = SYS_SYNC_FIFO
        0x00, 0x00, 0x00, 0x20, // recursive = SYS_SYNC_NOT_RECURSIVE
        0x00, 0x00, 0x02, 0x00, // pshared = SYS_SYNC_NOT_PROCESS_SHARED
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
    assert!(!host.mutexes().lookup(id).unwrap().attrs().recursive);
    let first = host.dispatch(
        Lv2Request::MutexLock {
            mutex_id: id,
            timeout: 0,
        },
        src,
        &rt,
    );
    assert!(matches!(first, Lv2Dispatch::Immediate { code: 0, .. }));
    let relock = host.dispatch(
        Lv2Request::MutexLock {
            mutex_id: id,
            timeout: 0,
        },
        src,
        &rt,
    );
    match relock {
        Lv2Dispatch::Immediate { code, .. } => {
            assert_eq!(code, u64::from(cellgov_ps3_abi::cell_errors::CELL_EDEADLK));
        }
        other => panic!("expected Immediate(EDEADLK), got {other:?}"),
    }
}

/// Build a `FakeRuntime` with a 28-byte `sys_mutex_attribute_t`
/// prefix (protocol, recursive, pshared, adaptive, ipc_key, flags)
/// committed at 0x200.
fn mutex_attr_runtime(
    protocol: u32,
    recursive: u32,
    pshared: u32,
    ipc_key: u64,
    flags: u32,
) -> FakeRuntime {
    let mut mem = cellgov_mem::GuestMemory::new(0x10000);
    let mut attr = [0u8; 28];
    attr[0..4].copy_from_slice(&protocol.to_be_bytes());
    attr[4..8].copy_from_slice(&recursive.to_be_bytes());
    attr[8..12].copy_from_slice(&pshared.to_be_bytes());
    attr[16..24].copy_from_slice(&ipc_key.to_be_bytes());
    attr[24..28].copy_from_slice(&flags.to_be_bytes());
    mem.apply_commit(
        cellgov_mem::ByteRange::new(cellgov_mem::GuestAddr::new(0x200), 28).unwrap(),
        &attr,
    )
    .unwrap();
    FakeRuntime::with_memory(mem)
}

fn create_with_attr_at_0x200(host: &mut Lv2Host, rt: &FakeRuntime) -> Lv2Dispatch {
    let src = UnitId::new(0);
    host.dispatch(
        Lv2Request::MutexCreate {
            id_ptr: 0x100,
            attr_ptr: 0x200,
        },
        src,
        rt,
    )
}

#[test]
fn an_unknown_mutex_protocol_is_einval_not_a_default() {
    // RPCS3 sys_mutex.cpp sys_mutex_create: protocol outside
    // FIFO / PRIORITY / PRIORITY_INHERIT is EINVAL.
    let rt = mutex_attr_runtime(0x20, 0x10, 0x200, 0, 0);
    let mut host = Lv2Host::new();
    seed_primary_ppu(&mut host, UnitId::new(0));
    let r = create_with_attr_at_0x200(&mut host, &rt);
    assert_eq!(r, Lv2Dispatch::immediate(cell_errors::CELL_EINVAL.into()));
}

#[test]
fn an_unknown_mutex_recursive_value_is_einval() {
    // RPCS3 sys_mutex.cpp sys_mutex_create: recursive outside
    // SYS_SYNC_RECURSIVE / SYS_SYNC_NOT_RECURSIVE is EINVAL.
    let rt = mutex_attr_runtime(0x1, 0x11, 0x200, 0, 0);
    let mut host = Lv2Host::new();
    seed_primary_ppu(&mut host, UnitId::new(0));
    let r = create_with_attr_at_0x200(&mut host, &rt);
    assert_eq!(r, Lv2Dispatch::immediate(cell_errors::CELL_EINVAL.into()));
}

#[test]
fn an_unknown_mutex_pshared_value_is_einval() {
    // RPCS3 sys_sync.h lv2_obj::create: pshared outside 0x100 /
    // 0x200 is EINVAL.
    let rt = mutex_attr_runtime(0x1, 0x20, 0, 0, 0);
    let mut host = Lv2Host::new();
    seed_primary_ppu(&mut host, UnitId::new(0));
    let r = create_with_attr_at_0x200(&mut host, &rt);
    assert_eq!(r, Lv2Dispatch::immediate(cell_errors::CELL_EINVAL.into()));
}

#[test]
fn a_process_shared_mutex_with_zero_ipc_key_is_einval() {
    // RPCS3 sys_sync.h lv2_obj::create: PROCESS_SHARED with a zero
    // ipc_key is EINVAL.
    let rt = mutex_attr_runtime(0x1, 0x20, 0x100, 0, 1);
    let mut host = Lv2Host::new();
    seed_primary_ppu(&mut host, UnitId::new(0));
    let r = create_with_attr_at_0x200(&mut host, &rt);
    assert_eq!(r, Lv2Dispatch::immediate(cell_errors::CELL_EINVAL.into()));
}

#[test]
fn a_process_shared_mutex_with_bad_attach_flags_is_einval() {
    // RPCS3 sys_sync.h lv2_obj::create: attach flags outside
    // NEWLY_CREATED / NOT_CREATE / NOT_CARE (1 / 2 / 3) are EINVAL
    // for a PROCESS_SHARED create.
    let rt = mutex_attr_runtime(0x1, 0x20, 0x100, 0xCAFE, 0);
    let mut host = Lv2Host::new();
    seed_primary_ppu(&mut host, UnitId::new(0));
    let r = create_with_attr_at_0x200(&mut host, &rt);
    assert_eq!(r, Lv2Dispatch::immediate(cell_errors::CELL_EINVAL.into()));
}

#[test]
fn a_valid_process_shared_mutex_creates_locally_and_reports_the_gap() {
    let rt = mutex_attr_runtime(0x1, 0x20, 0x100, 0xCAFE, 1);
    let mut host = Lv2Host::new();
    seed_primary_ppu(&mut host, UnitId::new(0));
    let r = create_with_attr_at_0x200(&mut host, &rt);
    assert!(matches!(r, Lv2Dispatch::Immediate { code: 0, .. }));
    assert_eq!(
        host.invariant_break_site_count("mutex.pshared_attach_unmodeled"),
        1,
        "the unmodeled cross-process attach must be reported, not silent",
    );
}

#[test]
fn a_null_mutex_id_ptr_is_efault_before_any_state_change() {
    // RPCS3 sys_mutex.cpp sys_mutex_create rejects a null mutex_id
    // before creation.
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let src = UnitId::new(0);
    seed_primary_ppu(&mut host, src);
    let pre = host.state_hash();
    let r = host.dispatch(
        Lv2Request::MutexCreate {
            id_ptr: 0,
            attr_ptr: 0,
        },
        src,
        &rt,
    );
    assert_eq!(r, Lv2Dispatch::immediate(cell_errors::CELL_EFAULT.into()));
    assert_eq!(
        pre,
        host.state_hash(),
        "an EFAULT create must not mint an id or a table entry",
    );
}

#[test]
fn an_unreadable_mutex_attr_ptr_is_efault_not_default() {
    // The kernel reads the attribute struct unconditionally; an
    // unreadable pointer is a guest fault, not the default-attribute
    // arm (RPCS3 sys_mutex.cpp sys_mutex_create).
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x100);
    let src = UnitId::new(0);
    seed_primary_ppu(&mut host, src);
    let r = host.dispatch(
        Lv2Request::MutexCreate {
            id_ptr: 0x10,
            attr_ptr: 0xDEAD_0000,
        },
        src,
        &rt,
    );
    assert_eq!(r, Lv2Dispatch::immediate(cell_errors::CELL_EFAULT.into()));
}

#[test]
fn a_recursive_relock_returns_ok_and_needs_matching_unlocks_before_the_waiter_wakes() {
    // RPCS3 sys_mutex.h lv2_mutex::try_lock: owner re-lock on a
    // SYS_SYNC_RECURSIVE mutex is CELL_OK with a count bump; RPCS3
    // sys_mutex.cpp sys_mutex_unlock releases (and grants to a
    // waiter) only once the count is back at zero.
    let rt = mutex_attr_runtime(0x1, 0x10, 0x200, 0, 0);
    let mut host = Lv2Host::new();
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
    let created = create_with_attr_at_0x200(&mut host, &rt);
    let id = match &created {
        Lv2Dispatch::Immediate {
            code: 0,
            effects: e,
        } => extract_write_u32(&e[0]),
        other => panic!("expected Immediate(0), got {other:?}"),
    };
    let lock = Lv2Request::MutexLock {
        mutex_id: id,
        timeout: 0,
    };
    assert!(matches!(
        host.dispatch(lock, owner_unit, &rt),
        Lv2Dispatch::Immediate { code: 0, .. }
    ));
    assert!(
        matches!(
            host.dispatch(lock, owner_unit, &rt),
            Lv2Dispatch::Immediate { code: 0, .. }
        ),
        "owner re-lock on a recursive mutex must return CELL_OK"
    );
    assert_eq!(host.mutexes().lookup(id).unwrap().lock_count(), 1);
    assert!(matches!(
        host.dispatch(lock, waiter_unit, &rt),
        Lv2Dispatch::Block { .. }
    ));
    let first_unlock = host.dispatch(Lv2Request::MutexUnlock { mutex_id: id }, owner_unit, &rt);
    assert!(
        matches!(first_unlock, Lv2Dispatch::Immediate { code: 0, .. }),
        "the first unlock consumes the recursive hold without waking, got {first_unlock:?}"
    );
    assert_eq!(
        host.mutexes().lookup(id).unwrap().owner(),
        Some(PpuThreadId::PRIMARY),
    );
    let second_unlock = host.dispatch(Lv2Request::MutexUnlock { mutex_id: id }, owner_unit, &rt);
    match second_unlock {
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
fn a_saturated_recursive_lock_count_returns_ekresource() {
    // RPCS3 sys_mutex.h lv2_mutex::try_lock: a lock count at u32::MAX
    // refuses the re-lock with CELL_EKRESOURCE and stays owned.
    let rt = mutex_attr_runtime(0x1, 0x10, 0x200, 0, 0);
    let mut host = Lv2Host::new();
    let src = UnitId::new(0);
    seed_primary_ppu(&mut host, src);
    let created = create_with_attr_at_0x200(&mut host, &rt);
    let id = match &created {
        Lv2Dispatch::Immediate {
            code: 0,
            effects: e,
        } => extract_write_u32(&e[0]),
        other => panic!("expected Immediate(0), got {other:?}"),
    };
    let lock = Lv2Request::MutexLock {
        mutex_id: id,
        timeout: 0,
    };
    assert!(matches!(
        host.dispatch(lock, src, &rt),
        Lv2Dispatch::Immediate { code: 0, .. }
    ));
    host.mutexes_mut().set_lock_count_for_test(id, u32::MAX);
    let relock = host.dispatch(lock, src, &rt);
    let Lv2Dispatch::Immediate { code, .. } = relock else {
        panic!("expected Immediate EKRESOURCE, got {relock:?}");
    };
    assert_eq!(code, cell_errors::CELL_EKRESOURCE.into());
    assert_eq!(host.mutexes().lookup(id).unwrap().lock_count(), u32::MAX);
    assert_eq!(
        host.mutexes().lookup(id).unwrap().owner(),
        Some(PpuThreadId::PRIMARY),
    );
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
    let Lv2Dispatch::Immediate { code, .. } = r else {
        panic!("expected Immediate, got {r:?}");
    };
    assert_eq!(code, cell_errors::CELL_EBUSY.into());
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
    let Lv2Dispatch::Immediate { code, .. } = r else {
        panic!("expected Immediate, got {r:?}");
    };
    assert_eq!(code, cell_errors::CELL_ESRCH.into());
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
    let Lv2Dispatch::Immediate { code, .. } = r else {
        panic!("expected Immediate, got {r:?}");
    };
    assert_eq!(code, cell_errors::CELL_EPERM.into());
}

#[test]
fn mutex_lock_from_transient_unit_without_alias_returns_esrch() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(256);
    let primary = UnitId::new(0);
    seed_primary_ppu(&mut host, primary);
    let created = host.dispatch(
        Lv2Request::MutexCreate {
            id_ptr: 0x100,
            attr_ptr: 0,
        },
        primary,
        &rt,
    );
    let id = match &created {
        Lv2Dispatch::Immediate {
            code: 0,
            effects: e,
        } => extract_write_u32(&e[0]),
        other => panic!("expected Immediate(0), got {other:?}"),
    };
    // Transient UnitId not in ppu_threads -> strict ESRCH.
    let transient = UnitId::new(99);
    let r = host.dispatch(
        Lv2Request::MutexLock {
            mutex_id: id,
            timeout: 0,
        },
        transient,
        &rt,
    );
    let Lv2Dispatch::Immediate { code, .. } = r else {
        panic!("expected Immediate ESRCH, got {r:?}");
    };
    assert_eq!(code, cell_errors::CELL_ESRCH.into());
}

#[test]
fn mutex_lock_from_aliased_transient_unit_acquires_as_primary() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(256);
    let primary = UnitId::new(0);
    seed_primary_ppu(&mut host, primary);
    let created = host.dispatch(
        Lv2Request::MutexCreate {
            id_ptr: 0x100,
            attr_ptr: 0,
        },
        primary,
        &rt,
    );
    let id = match &created {
        Lv2Dispatch::Immediate {
            code: 0,
            effects: e,
        } => extract_write_u32(&e[0]),
        other => panic!("expected Immediate(0), got {other:?}"),
    };
    let transient = UnitId::new(99);
    assert!(host.alias_unit_to_primary(transient));
    let r = host.dispatch(
        Lv2Request::MutexLock {
            mutex_id: id,
            timeout: 0,
        },
        transient,
        &rt,
    );
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
    // Dropping the alias restores the strict ESRCH path for the
    // retired UnitId.
    let other_transient = UnitId::new(100);
    assert!(host.alias_unit_to_primary(other_transient));
    assert!(host.drop_ppu_thread_alias(other_transient));
    let after_drop = host.dispatch(
        Lv2Request::MutexLock {
            mutex_id: id,
            timeout: 0,
        },
        other_transient,
        &rt,
    );
    let Lv2Dispatch::Immediate { code, .. } = after_drop else {
        panic!("expected Immediate ESRCH, got {after_drop:?}");
    };
    assert_eq!(code, cell_errors::CELL_ESRCH.into());
}
