//! LV2 mutex dispatch tests: attribute decoding, contended block-and-wake, trylock EBUSY, EPERM non-owner unlock, and transient-unit aliasing.

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
        0x00, 0x00, 0x00, 0x20, // protocol = 0x20
        0x00, 0x00, 0x00, 0x11, // recursive
        0x00, 0x00, 0x00, 0x00, // pshared
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

// Alias-path coverage for the (1c) module_start fix
// (`docs/dev/bug_investigations/fix_walk_runtime_lv2_host_asymmetry.md`).
// Pair: a transient unit WITHOUT an alias entry must still ESRCH
// on sync syscall (strict lookup preserved); an aliased transient
// unit's lock acquires against a host-known mutex (alias works).

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
    // Acquired against the PRIMARY id (alias target), not against
    // a fabricated thread for the transient unit.
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
