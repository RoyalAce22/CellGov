//! Cross-primitive dispatch tests: shapes that touch more than
//! one LV2 primitive at once. Per-primitive tests live next to
//! their dispatch code in `src/host/<primitive>.rs`; host-scope
//! tests (construction, state_hash gating, stub syscalls) live
//! in `src/host.rs`.

use super::*;
use crate::host::test_support::*;

#[test]
fn lwmutex_and_mutex_id_spaces_are_independent() {
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
    // shared `next_kernel_id` allocator (0x4000_0001+).
    assert_eq!(lw_id, 1);
    assert!(hv_id >= 0x4000_0001);
    // Drive the heavy mutex's lock/unlock cycle and verify the
    // lwmutex table is untouched (no cross-contamination via the
    // shared allocator). The kernel-side lwmutex lock semantics
    // intentionally always park, so we exercise lwmutex via the
    // kernel signal-only path: an unlock against an empty queue
    // sets the signal.
    host.dispatch(
        Lv2Request::MutexLock {
            mutex_id: hv_id,
            timeout: 0,
        },
        src,
        &rt,
    );
    assert_eq!(
        host.mutexes().lookup(hv_id).unwrap().owner(),
        Some(PpuThreadId::PRIMARY),
    );
    assert!(!host.lwmutexes().lookup(lw_id).unwrap().signaled());
    host.dispatch(Lv2Request::LwMutexUnlock { id: lw_id }, src, &rt);
    assert!(host.lwmutexes().lookup(lw_id).unwrap().signaled());
    assert_eq!(
        host.mutexes().lookup(hv_id).unwrap().owner(),
        Some(PpuThreadId::PRIMARY),
    );
    host.dispatch(Lv2Request::MutexUnlock { mutex_id: hv_id }, src, &rt);
    assert_eq!(host.mutexes().lookup(hv_id).unwrap().owner(), None);
    assert!(host.lwmutexes().lookup(lw_id).unwrap().signaled());
}

#[test]
fn lwmutex_and_mutex_waiter_lists_do_not_cross_contaminate() {
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
    // Park `waiter_tid` directly on the lwmutex sleep queue so the
    // unlock has a transfer target. The kernel-side dispatch
    // doesn't care about ownership; the HLE-side fast path already
    // covered that, so a direct enqueue is the legitimate way to
    // exercise the cross-table independence here.
    host.lwmutexes_mut()
        .enqueue_waiter(lw_id, waiter_tid)
        .unwrap();
    host.dispatch(
        Lv2Request::MutexLock {
            mutex_id: hv_id,
            timeout: 0,
        },
        owner_unit,
        &rt,
    );
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
    let r = host.dispatch(Lv2Request::MutexUnlock { mutex_id: hv_id }, owner_unit, &rt);
    assert!(matches!(
        r,
        Lv2Dispatch::Immediate {
            code: 0,
            effects: _,
        }
    ));
    // The heavy-mutex unlock did not touch the lwmutex sleep queue.
    assert_eq!(
        host.lwmutexes()
            .lookup(lw_id)
            .unwrap()
            .waiters()
            .iter()
            .collect::<Vec<_>>(),
        vec![waiter_tid],
    );
    let r = host.dispatch(Lv2Request::LwMutexUnlock { id: lw_id }, owner_unit, &rt);
    match r {
        Lv2Dispatch::WakeAndReturn { woken_unit_ids, .. } => {
            assert_eq!(woken_unit_ids, vec![waiter_unit]);
        }
        other => panic!("expected WakeAndReturn, got {other:?}"),
    }
}

#[test]
fn multi_primitive_determinism_canary() {
    fn canonical_run() -> Vec<(String, u64)> {
        let mut host = Lv2Host::new();
        let rt = fake_runtime_with_valid_sync_attr(0x10000);
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
                attr_ptr: VALID_SYNC_ATTR_PTR,
                initial: 0,
                max: 4,
            },
            u0,
            &rt,
        ) {
            Lv2Dispatch::Immediate { effects: e, .. } => extract_write_u32(&e[0]),
            other => panic!("unexpected {other:?}"),
        };

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
                    mutex_ptr: 0,
                    timeout: 0,
                },
            ),
            (
                "t1-lw-lock",
                u1,
                Lv2Request::LwMutexLock {
                    id: lwmutex_id,
                    mutex_ptr: 0,
                    timeout: 0,
                },
            ),
            (
                "t2-lw-lock",
                u2,
                Lv2Request::LwMutexLock {
                    id: lwmutex_id,
                    mutex_ptr: 0,
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
            // Tag omits effect payloads; scheduler selection
            // order is already covered by the post-dispatch
            // state hash paired with this tag.
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
    // Guard against accidentally neutering the script.
    assert!(run_a.len() >= 15);
}

// Lost-wake family: lwmutex, mutex, semaphore, event queue, and
// event flag all remember a release issued before the
// corresponding wait. Cond does not (see
// `cond_signal_before_wait_does_not_wake_subsequent_waiter` in
// `src/host/cond.rs`).

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
    // An unlock against an empty kernel sleep queue sets the signal
    // so the next contended lock can pass. This guards the
    // lost-wake case where a release races ahead of an acquire.
    let unlock = host.dispatch(Lv2Request::LwMutexUnlock { id }, owner_unit, &rt);
    assert!(matches!(
        unlock,
        Lv2Dispatch::Immediate {
            code: 0,
            effects: _,
        }
    ));
    let lock = host.dispatch(
        Lv2Request::LwMutexLock {
            id,
            mutex_ptr: 0,
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
    let rt = fake_runtime_with_valid_sync_attr(0x10000);
    let src = UnitId::new(0);
    seed_primary_ppu(&mut host, src);
    let created = host.dispatch(
        Lv2Request::SemaphoreCreate {
            id_ptr: 0x100,
            attr_ptr: VALID_SYNC_ATTR_PTR,
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
    let rt = fake_runtime_with_valid_sync_attr(0x10000);
    let src = UnitId::new(0);
    seed_primary_ppu(&mut host, src);
    let created = host.dispatch(
        Lv2Request::EventFlagCreate {
            id_ptr: 0x100,
            attr_ptr: VALID_SYNC_ATTR_PTR,
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
