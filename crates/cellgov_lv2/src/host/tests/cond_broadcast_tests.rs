use super::*;

#[test]
fn cond_signal_all_wakes_first_reparks_rest_when_mutex_free() {
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
    assert_eq!(host.conds().lookup(cond_id).unwrap().waiters().len(), 3);
    assert_eq!(host.mutexes().lookup(mutex_id).unwrap().owner(), None);
    let r = host.dispatch(Lv2Request::CondSignalAll { id: cond_id }, signaler, &rt);
    match r {
        Lv2Dispatch::WakeAndReturn {
            woken_unit_ids,
            response_updates,
            ..
        } => {
            assert_eq!(woken_unit_ids, vec![w1]);
            let updated_units: Vec<_> = response_updates.iter().map(|(u, _)| *u).collect();
            assert_eq!(updated_units, vec![w1, w2, w3]);
        }
        other => panic!("expected WakeAndReturn, got {other:?}"),
    }
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
    let (mut host, rt, w1) = cond_fixture();
    let w2 = UnitId::new(1);
    let w3 = UnitId::new(2);
    let holder = UnitId::new(3);
    let signaler = UnitId::new(4);
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
    let (mut host, rt, src) = cond_fixture();
    let r = host.dispatch(Lv2Request::CondSignalAll { id: 0xDEAD }, src, &rt);
    let Lv2Dispatch::Immediate { code, .. } = r else {
        panic!("expected Immediate, got {r:?}");
    };
    assert_eq!(code, errno::CELL_ESRCH.into());
}

#[test]
#[cfg(not(debug_assertions))]
fn cond_signal_all_flags_invariant_break_on_double_parked_waker() {
    let (mut host, rt, waker_unit) = cond_fixture();
    let signaler_unit = UnitId::new(1);
    host.ppu_threads_mut()
        .create(signaler_unit, primary_attrs())
        .expect("signaler create");
    let mutex_id = create_mutex_host(&mut host, waker_unit, &rt);
    host.dispatch(
        Lv2Request::MutexLock {
            mutex_id,
            timeout: 0,
        },
        signaler_unit,
        &rt,
    );
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
    host.conds_mut()
        .enqueue_waiter(cond_id, PpuThreadId::PRIMARY)
        .unwrap();
    host.mutexes_mut()
        .enqueue_waiter(mutex_id, PpuThreadId::PRIMARY)
        .unwrap();
    let breaks_before = host.observability().invariant_break_count;
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
            assert_eq!(woken_unit_ids, vec![waker_unit]);
            assert_eq!(response_updates.len(), 1);
            assert_eq!(response_updates[0].0, waker_unit);
            assert!(matches!(
                response_updates[0].1,
                PendingResponse::ReturnCode { code } if code == errno::CELL_ESRCH.into()
            ));
        }
        other => panic!("expected WakeAndReturn, got {other:?}"),
    }
    assert!(host.observability().invariant_break_count > breaks_before);
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
fn cond_signal_before_wait_does_not_wake_subsequent_waiter() {
    for variant in ["signal_one", "signal_all", "signal_to"] {
        let (mut host, rt, waiter_unit) = cond_fixture();
        let signaler_unit = UnitId::new(1);
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
        match variant {
            "signal_to" => {
                let Lv2Dispatch::Immediate { code, .. } = pre_signal else {
                    panic!("{variant}: expected Immediate, got {pre_signal:?}");
                };
                assert_eq!(
                    code,
                    errno::CELL_EPERM.into(),
                    "{variant}: signal_to on target-not-parked must EPERM",
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
    let (mut host, rt, waiter_unit) = cond_fixture();
    let signaler_unit = UnitId::new(1);
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
fn cond_signal_dispatches_witness_counts_invocations() {
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
    assert_eq!(host.observability().cond_signal_dispatches, 0);
    host.dispatch(Lv2Request::CondSignal { id: cond_id }, src, &rt);
    host.dispatch(Lv2Request::CondSignal { id: cond_id }, src, &rt);
    assert_eq!(host.observability().cond_signal_dispatches, 2);
}
