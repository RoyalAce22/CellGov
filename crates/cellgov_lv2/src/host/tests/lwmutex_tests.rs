//! LV2 lwmutex dispatch tests: kernel-side parking, trylock EBUSY, FIFO unlock transfer, and EDEADLK on duplicate parks.

use super::*;
use crate::host::test_support::{extract_write_u32, seed_primary_ppu, FakeRuntime};
use crate::ppu_thread::{PpuThreadAttrs, PpuThreadId};
use crate::request::Lv2Request;

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
    let Lv2Dispatch::Immediate { code, .. } = r else {
        panic!("expected Immediate, got {r:?}");
    };
    assert_eq!(code, cell_errors::CELL_ESRCH.into());
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
        .enqueue_waiter(id, PpuThreadId::new(0x0100_0002))
        .unwrap();
    let r = host.dispatch(Lv2Request::LwMutexDestroy { id }, source, &rt);
    let Lv2Dispatch::Immediate { code, .. } = r else {
        panic!("expected Immediate, got {r:?}");
    };
    assert_eq!(code, cell_errors::CELL_EBUSY.into());
    assert!(host.lwmutexes().lookup(id).is_some());
}

#[test]
fn lwmutex_lock_unknown_id_returns_esrch() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let src = UnitId::new(0);
    seed_primary_ppu(&mut host, src);
    let r = host.dispatch(
        Lv2Request::LwMutexLock {
            id: 99,
            mutex_ptr: 0,
            timeout: 0,
        },
        src,
        &rt,
    );
    let Lv2Dispatch::Immediate { code, .. } = r else {
        panic!("expected Immediate, got {r:?}");
    };
    assert_eq!(code, cell_errors::CELL_ESRCH.into());
}

#[test]
fn lwmutex_lock_on_fresh_entry_parks_kernel_side() {
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
    let r = host.dispatch(
        Lv2Request::LwMutexLock {
            id,
            mutex_ptr: 0,
            timeout: 0,
        },
        src,
        &rt,
    );
    match r {
        Lv2Dispatch::Block { .. } => {}
        other => panic!("expected Block, got {other:?}"),
    }
    let entry = host.lwmutexes().lookup(id).unwrap();
    assert!(!entry.signaled());
    assert_eq!(entry.waiters().len(), 1);
}

#[test]
fn lwmutex_lock_contended_parks_caller_on_waiter_list() {
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
    host.dispatch(
        Lv2Request::LwMutexLock {
            id,
            mutex_ptr: 0,
            timeout: 0,
        },
        owner_unit,
        &rt,
    );
    let r = host.dispatch(
        Lv2Request::LwMutexLock {
            id,
            mutex_ptr: 0,
            timeout: 0,
        },
        waiter_unit,
        &rt,
    );
    match r {
        Lv2Dispatch::Block {
            reason: crate::dispatch::Lv2BlockReason::LwMutex { id: blocked_id },
            pending: PendingResponse::LwMutexWake { mutex_ptr: 0, .. },
            effects: _,
        } => {
            assert_eq!(blocked_id, id);
        }
        other => panic!("expected Block on LwMutex, got {other:?}"),
    }
    let entry = host.lwmutexes().lookup(id).unwrap();
    assert!(!entry.signaled());
    let seen: Vec<_> = entry.waiters().iter().collect();
    assert_eq!(seen.len(), 2);
    assert!(seen.contains(&waiter_tid));
}

#[test]
fn lwmutex_trylock_on_fresh_entry_returns_ebusy_kernel_side() {
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
    let Lv2Dispatch::Immediate { code, .. } = r else {
        panic!("expected Immediate, got {r:?}");
    };
    assert_eq!(code, cell_errors::CELL_EBUSY.into());
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
    host.dispatch(
        Lv2Request::LwMutexLock {
            id,
            mutex_ptr: 0,
            timeout: 0,
        },
        owner_unit,
        &rt,
    );
    let r = host.dispatch(Lv2Request::LwMutexTryLock { id }, other_unit, &rt);
    let Lv2Dispatch::Immediate { code, .. } = r else {
        panic!("expected Immediate, got {r:?}");
    };
    assert_eq!(code, cell_errors::CELL_EBUSY.into());
    let entry = host.lwmutexes().lookup(id).unwrap();
    assert!(!entry.signaled());
    assert_eq!(entry.waiters().len(), 1);
}

#[test]
fn lwmutex_trylock_unknown_returns_esrch() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let src = UnitId::new(0);
    seed_primary_ppu(&mut host, src);
    let r = host.dispatch(Lv2Request::LwMutexTryLock { id: 77 }, src, &rt);
    let Lv2Dispatch::Immediate { code, .. } = r else {
        panic!("expected Immediate, got {r:?}");
    };
    assert_eq!(code, cell_errors::CELL_ESRCH.into());
}

#[test]
fn lwmutex_unlock_without_waiters_signals() {
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
    let r = host.dispatch(Lv2Request::LwMutexUnlock { id }, src, &rt);
    assert!(matches!(
        r,
        Lv2Dispatch::Immediate {
            code: 0,
            effects: _,
        }
    ));
    assert!(host.lwmutexes().lookup(id).unwrap().signaled());
}

#[test]
fn lwmutex_unlock_with_waiters_transfers_to_head() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let unlocker = UnitId::new(0);
    let waiter_unit = UnitId::new(1);
    seed_primary_ppu(&mut host, unlocker);
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
        unlocker,
        &rt,
    );
    let id = match &created {
        Lv2Dispatch::Immediate {
            code: 0,
            effects: e,
        } => extract_write_u32(&e[0]),
        other => panic!("expected Immediate(0), got {other:?}"),
    };
    host.lwmutexes_mut().enqueue_waiter(id, waiter_tid).unwrap();
    let r = host.dispatch(Lv2Request::LwMutexUnlock { id }, unlocker, &rt);
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
    let entry = host.lwmutexes().lookup(id).unwrap();
    assert!(!entry.signaled());
    assert!(entry.waiters().is_empty());
}

#[test]
fn lwmutex_unlock_signals_when_only_blocked_caller_is_unlocker() {
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
    let r = host.dispatch(Lv2Request::LwMutexUnlock { id }, other_unit, &rt);
    let Lv2Dispatch::Immediate { code, .. } = r else {
        panic!("expected Immediate, got {r:?}");
    };
    assert_eq!(code, 0);
    assert!(host.lwmutexes().lookup(id).unwrap().signaled());
}

#[test]
fn lwmutex_unlock_unknown_returns_esrch() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let src = UnitId::new(0);
    seed_primary_ppu(&mut host, src);
    let r = host.dispatch(Lv2Request::LwMutexUnlock { id: 99 }, src, &rt);
    let Lv2Dispatch::Immediate { code, .. } = r else {
        panic!("expected Immediate, got {r:?}");
    };
    assert_eq!(code, cell_errors::CELL_ESRCH.into());
}

#[test]
fn lwmutex_unlock_with_three_waiters_wakes_head_in_fifo_order() {
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
    let _ = (t1, t2, t3);
    host.lwmutexes_mut().enqueue_waiter(id, t1).unwrap();
    host.lwmutexes_mut().enqueue_waiter(id, t2).unwrap();
    host.lwmutexes_mut().enqueue_waiter(id, t3).unwrap();
    match host.dispatch(Lv2Request::LwMutexUnlock { id }, u0, &rt) {
        Lv2Dispatch::WakeAndReturn { woken_unit_ids, .. } => {
            assert_eq!(woken_unit_ids, vec![u1]);
        }
        other => panic!("expected WakeAndReturn, got {other:?}"),
    }
    match host.dispatch(Lv2Request::LwMutexUnlock { id }, u0, &rt) {
        Lv2Dispatch::WakeAndReturn { woken_unit_ids, .. } => {
            assert_eq!(woken_unit_ids, vec![u2]);
        }
        other => panic!("expected WakeAndReturn, got {other:?}"),
    }
    match host.dispatch(Lv2Request::LwMutexUnlock { id }, u0, &rt) {
        Lv2Dispatch::WakeAndReturn { woken_unit_ids, .. } => {
            assert_eq!(woken_unit_ids, vec![u3]);
        }
        other => panic!("expected WakeAndReturn, got {other:?}"),
    }
    match host.dispatch(Lv2Request::LwMutexUnlock { id }, u0, &rt) {
        Lv2Dispatch::Immediate { code: 0, .. } => {}
        other => panic!("expected Immediate(0), got {other:?}"),
    }
    assert!(host.lwmutexes().lookup(id).unwrap().signaled());
    assert!(host.lwmutexes().lookup(id).unwrap().waiters().is_empty());
}

#[test]
fn lwmutex_lock_duplicate_park_returns_edeadlk() {
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
    host.dispatch(
        Lv2Request::LwMutexLock {
            id,
            mutex_ptr: 0,
            timeout: 0,
        },
        owner_unit,
        &rt,
    );
    host.dispatch(
        Lv2Request::LwMutexLock {
            id,
            mutex_ptr: 0,
            timeout: 0,
        },
        waiter_unit,
        &rt,
    );
    let r = host.dispatch(
        Lv2Request::LwMutexLock {
            id,
            mutex_ptr: 0,
            timeout: 0,
        },
        waiter_unit,
        &rt,
    );
    let Lv2Dispatch::Immediate { code, .. } = r else {
        panic!("expected Immediate, got {r:?}");
    };
    assert_eq!(code, cell_errors::CELL_EDEADLK.into());
}
