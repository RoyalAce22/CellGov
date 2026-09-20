use super::*;

#[test]
fn cond_signal_no_waiter_is_observably_lost() {
    let (mut host, rt, src) = cond_fixture();
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
    assert!(host.conds().lookup(cond_id).unwrap().waiters().is_empty());
}

#[test]
fn cond_signal_unknown_id_returns_esrch() {
    let (mut host, rt, src) = cond_fixture();
    let r = host.dispatch(Lv2Request::CondSignal { id: 0xDEAD }, src, &rt);
    let Lv2Dispatch::Immediate { code, .. } = r else {
        panic!("expected Immediate, got {r:?}");
    };
    assert_eq!(code, errno::CELL_ESRCH.into());
}

#[test]
fn cond_signal_wakes_waiter_cleanly_when_mutex_free() {
    let (mut host, rt, waiter_unit) = cond_fixture();
    let signaler_unit = UnitId::new(1);
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
    assert_eq!(
        host.mutexes().lookup(mutex_id).unwrap().owner(),
        Some(PpuThreadId::PRIMARY),
    );
    assert!(host.conds().lookup(cond_id).unwrap().waiters().is_empty());
}

#[test]
fn cond_signal_reparks_waiter_on_mutex_when_held() {
    let (mut host, rt, waiter_unit) = cond_fixture();
    let third_unit = UnitId::new(1);
    let signaler_unit = UnitId::new(2);
    let third_tid = host
        .ppu_threads_mut()
        .create(third_unit, primary_attrs())
        .expect("third create");
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
    assert_eq!(
        host.mutexes().lookup(mutex_id).unwrap().owner(),
        Some(third_tid),
    );
    assert_eq!(
        host.mutexes()
            .lookup(mutex_id)
            .unwrap()
            .waiters()
            .iter()
            .collect::<Vec<_>>(),
        vec![PpuThreadId::PRIMARY],
    );
    assert!(host.conds().lookup(cond_id).unwrap().waiters().is_empty());
}

#[test]
fn cond_signal_wakes_fifo_head_when_multiple_waiters() {
    let (mut host, rt, w1_unit) = cond_fixture();
    let w2_unit = UnitId::new(1);
    let signaler_unit = UnitId::new(2);
    let w2_tid = host
        .ppu_threads_mut()
        .create(w2_unit, primary_attrs())
        .expect("w2 create");
    host.ppu_threads_mut()
        .create(signaler_unit, primary_attrs())
        .expect("signaler create");
    let mutex_id = create_mutex_host(&mut host, w1_unit, &rt);
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
    let r = host.dispatch(Lv2Request::CondSignal { id: cond_id }, signaler_unit, &rt);
    match r {
        Lv2Dispatch::WakeAndReturn { woken_unit_ids, .. } => {
            assert_eq!(woken_unit_ids, vec![w1_unit]);
        }
        other => panic!("expected WakeAndReturn, got {other:?}"),
    }
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
fn cond_signal_to_targets_specific_waiter_and_preserves_order() {
    let (mut host, rt, w1) = cond_fixture();
    let w2 = UnitId::new(1);
    let w3 = UnitId::new(2);
    let signaler = UnitId::new(3);
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
    assert_eq!(host.mutexes().lookup(mutex_id).unwrap().owner(), None);
    assert_eq!(host.conds().lookup(cond_id).unwrap().waiters().len(), 3);
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
    assert_eq!(
        host.mutexes().lookup(mutex_id).unwrap().owner(),
        Some(w2_tid)
    );
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
fn cond_signal_to_target_not_waiting_returns_eperm() {
    let (mut host, rt, w1) = cond_fixture();
    let other = UnitId::new(1);
    let signaler = UnitId::new(2);
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
    let r = host.dispatch(
        Lv2Request::CondSignalTo {
            id: cond_id,
            target_thread: other_tid.raw() as u32,
        },
        signaler,
        &rt,
    );
    let Lv2Dispatch::Immediate { code, .. } = r else {
        panic!("expected Immediate, got {r:?}");
    };
    assert_eq!(code, errno::CELL_EPERM.into());
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
    let (mut host, rt, src) = cond_fixture();
    let r = host.dispatch(
        Lv2Request::CondSignalTo {
            id: 0xDEAD,
            target_thread: PpuThreadId::PRIMARY.raw() as u32,
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
fn cond_signal_to_reparks_target_on_mutex_when_held() {
    let (mut host, rt, w1) = cond_fixture();
    let w2 = UnitId::new(1);
    let holder = UnitId::new(2);
    let signaler = UnitId::new(3);
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
    assert_eq!(
        host.mutexes().lookup(mutex_id).unwrap().owner(),
        Some(holder_tid),
    );
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
        vec![w2_tid],
    );
}
