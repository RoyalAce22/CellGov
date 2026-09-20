use super::*;

#[test]
fn cond_wait_releases_mutex_and_parks_caller() {
    let (mut host, rt, src) = cond_fixture();
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
                crate::dispatch::Lv2BlockReason::Cond {
                    id,
                    mutex_id: m,
                    ..
                } if id == cond_id && m == mutex_id
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
    assert_eq!(host.mutexes().lookup(mutex_id).unwrap().owner(), None);
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
    let (mut host, rt, owner_unit) = cond_fixture();
    let waiter_unit = UnitId::new(1);
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
    assert_eq!(
        host.mutexes().lookup(mutex_id).unwrap().owner(),
        Some(waiter_tid),
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
}

#[test]
fn cond_wait_by_non_owner_returns_eperm() {
    let (mut host, rt, owner_unit) = cond_fixture();
    let other_unit = UnitId::new(1);
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
    let r = host.dispatch(
        Lv2Request::CondWait {
            id: cond_id,
            timeout: 0,
        },
        other_unit,
        &rt,
    );
    let Lv2Dispatch::Immediate { code, .. } = r else {
        panic!("expected Immediate, got {r:?}");
    };
    assert_eq!(code, errno::CELL_EPERM.into());
    assert_eq!(
        host.mutexes().lookup(mutex_id).unwrap().owner(),
        Some(PpuThreadId::PRIMARY),
    );
    assert!(host.conds().lookup(cond_id).unwrap().waiters().is_empty());
}

#[test]
fn cond_wait_unknown_id_returns_esrch() {
    let (mut host, rt, src) = cond_fixture();
    let r = host.dispatch(
        Lv2Request::CondWait {
            id: 0xDEAD,
            timeout: 0,
        },
        src,
        &rt,
    );
    let Lv2Dispatch::Immediate { code, .. } = r else {
        panic!("expected Immediate, got {r:?}");
    };
    assert_eq!(code, errno::CELL_ESRCH.into());
}

#[test]
fn cond_wait_ring_wake_fires_for_cond1_when_slot_ring_has_unconsumed_data() {
    // cond[1] of slot 0; slot 0 at base 0x2000: cursor 0 < limit 256.
    let rt = runtime_with_cond_attr(0x100, 0x8006_0100_0000_0040, &[(0x2004, 256), (0x2010, 0)]);
    let (mut host, rt, src) = cond_fixture_with(rt, UnitId::new(0));
    let mutex_id = create_mutex_host(&mut host, src, &rt);
    host.dispatch(
        Lv2Request::MutexLock {
            mutex_id,
            timeout: 0,
        },
        src,
        &rt,
    );
    let cond_id = create_cond_with_attr(&mut host, src, &rt, mutex_id);
    mark_seed_applied_at(&mut host, 0x2000);
    let r = host.dispatch(
        Lv2Request::CondWait {
            id: cond_id,
            timeout: 0,
        },
        src,
        &rt,
    );
    assert!(
        matches!(r, Lv2Dispatch::Immediate { code: 0, .. }),
        "ring-check arm must satisfy the wait immediately, got {r:?}",
    );
    assert_eq!(host.observability().cond_ring_wakes, 1);
    assert_eq!(host.observability().cond0_producer_waits(), 0);
    let entry = host.conds().lookup(cond_id).unwrap();
    assert!(entry.waiters().is_empty(), "caller must not park");
}

#[test]
fn cond_wait_slot1_cond1_reads_slot1_ring() {
    // cond[1] of slot 1; slot 0 depleted, slot 1 (base+0x8000) has data.
    let rt = runtime_with_cond_attr(
        0x100,
        0x8006_0100_0000_0041,
        &[(0x2004, 256), (0x2010, 256), (0xa004, 256), (0xa010, 0)],
    );
    let (mut host, rt, src) = cond_fixture_with(rt, UnitId::new(0));
    let mutex_id = create_mutex_host(&mut host, src, &rt);
    host.dispatch(
        Lv2Request::MutexLock {
            mutex_id,
            timeout: 0,
        },
        src,
        &rt,
    );
    let cond_id = create_cond_with_attr(&mut host, src, &rt, mutex_id);
    mark_seed_applied_at(&mut host, 0x2000);
    let r = host.dispatch(
        Lv2Request::CondWait {
            id: cond_id,
            timeout: 0,
        },
        src,
        &rt,
    );
    assert!(matches!(r, Lv2Dispatch::Immediate { code: 0, .. }));
    assert_eq!(host.observability().cond_ring_wakes, 1);
}

#[test]
fn cond_wait_cond1_depleted_ring_parks_without_witness() {
    // cond[1] of slot 0 with its ring depleted: cursor == limit.
    let rt = runtime_with_cond_attr(
        0x100,
        0x8006_0100_0000_0040,
        &[(0x2004, 256), (0x2010, 256)],
    );
    let (mut host, rt, src) = cond_fixture_with(rt, UnitId::new(0));
    let mutex_id = create_mutex_host(&mut host, src, &rt);
    host.dispatch(
        Lv2Request::MutexLock {
            mutex_id,
            timeout: 0,
        },
        src,
        &rt,
    );
    let cond_id = create_cond_with_attr(&mut host, src, &rt, mutex_id);
    mark_seed_applied_at(&mut host, 0x2000);
    let r = host.dispatch(
        Lv2Request::CondWait {
            id: cond_id,
            timeout: 0,
        },
        src,
        &rt,
    );
    assert!(
        matches!(r, Lv2Dispatch::Block { .. }),
        "depleted ring must park the caller, got {r:?}",
    );
    assert_eq!(host.observability().cond_ring_wakes, 0);
    assert_eq!(host.observability().cond0_producer_waits(), 0);
}

#[test]
fn cond_wait_cond0_parks_even_with_ring_data_and_counts_producer_wait() {
    // cond[0] of slot 0; ring HAS data -- the record-finish wait is
    // producer-fed and must not be satisfied by ring state.
    let rt = runtime_with_cond_attr(0x100, 0x8006_0100_0000_0030, &[(0x2004, 256), (0x2010, 0)]);
    let (mut host, rt, src) = cond_fixture_with(rt, UnitId::new(0));
    let mutex_id = create_mutex_host(&mut host, src, &rt);
    host.dispatch(
        Lv2Request::MutexLock {
            mutex_id,
            timeout: 0,
        },
        src,
        &rt,
    );
    let cond_id = create_cond_with_attr(&mut host, src, &rt, mutex_id);
    mark_seed_applied_at(&mut host, 0x2000);
    let r = host.dispatch(
        Lv2Request::CondWait {
            id: cond_id,
            timeout: 0,
        },
        src,
        &rt,
    );
    assert!(
        matches!(r, Lv2Dispatch::Block { .. }),
        "cond[0] must park; its wake is producer-fed, got {r:?}",
    );
    assert_eq!(host.observability().cond_ring_wakes, 0);
    assert_eq!(host.observability().cond0_producer_waits(), 1);
}

#[test]
fn cond_wait_keyless_cond_never_consults_the_ring() {
    // Ring would say "data available" -- but the cond is keyless.
    let rt = runtime_with_cond_attr(0x200, 0, &[(0x2004, 256), (0x2010, 0)]);
    let (mut host, rt, src) = cond_fixture_with(rt, UnitId::new(0));
    let mutex_id = create_mutex_host(&mut host, src, &rt);
    host.dispatch(
        Lv2Request::MutexLock {
            mutex_id,
            timeout: 0,
        },
        src,
        &rt,
    );
    let cond_id = create_cond_with_attr(&mut host, src, &rt, mutex_id);
    mark_seed_applied_at(&mut host, 0x2000);
    let r = host.dispatch(
        Lv2Request::CondWait {
            id: cond_id,
            timeout: 0,
        },
        src,
        &rt,
    );
    assert!(matches!(r, Lv2Dispatch::Block { .. }));
    assert_eq!(host.observability().cond_ring_wakes, 0);
}
