//! Cross-primitive dispatch tests.
//!
//! Tests that exercise more than one LV2 primitive at once, and
//! therefore would arbitrarily pick a home if placed in any one
//! submodule's tests block. Per-primitive tests live next to the
//! dispatch code they exercise (`src/host/<primitive>.rs`);
//! host-scope tests (construction, state_hash gating, stub
//! syscalls) live in `src/host.rs`.

use super::*;
use crate::host::test_support::*;

// ---------------------------------------------------------------
// Cross-primitive isolation.
//
// lwmutex and heavy mutex share neither their id space nor their
// waiter lists. A caller touching one primitive must not leak
// into the other's table state.
// ---------------------------------------------------------------

#[test]
fn lwmutex_and_mutex_id_spaces_are_independent() {
    // The two tables must not collide on ids: a lwmutex id and
    // a mutex id can legitimately share the same u32 value.
    // Acquiring one must not affect the other.
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
    // shared `next_kernel_id` allocator (0x4000_0001+). They
    // MUST NOT collide regardless of that layout; the table
    // types are distinct.
    assert_eq!(lw_id, 1);
    assert!(hv_id >= 0x4000_0001);
    // Acquire both with the primary.
    host.dispatch(
        Lv2Request::LwMutexLock {
            id: lw_id,
            timeout: 0,
        },
        src,
        &rt,
    );
    host.dispatch(
        Lv2Request::MutexLock {
            mutex_id: hv_id,
            timeout: 0,
        },
        src,
        &rt,
    );
    // Both owned by primary, independently.
    assert_eq!(
        host.lwmutexes().lookup(lw_id).unwrap().owner(),
        Some(PpuThreadId::PRIMARY),
    );
    assert_eq!(
        host.mutexes().lookup(hv_id).unwrap().owner(),
        Some(PpuThreadId::PRIMARY),
    );
    // Release lwmutex; heavy mutex unchanged.
    host.dispatch(Lv2Request::LwMutexUnlock { id: lw_id }, src, &rt);
    assert_eq!(host.lwmutexes().lookup(lw_id).unwrap().owner(), None);
    assert_eq!(
        host.mutexes().lookup(hv_id).unwrap().owner(),
        Some(PpuThreadId::PRIMARY),
    );
    // Release heavy mutex; lwmutex still free.
    host.dispatch(Lv2Request::MutexUnlock { mutex_id: hv_id }, src, &rt);
    assert_eq!(host.mutexes().lookup(hv_id).unwrap().owner(), None);
    assert_eq!(host.lwmutexes().lookup(lw_id).unwrap().owner(), None);
}

#[test]
fn lwmutex_and_mutex_waiter_lists_do_not_cross_contaminate() {
    // A thread parked on a lwmutex must not appear as a waiter
    // on a heavy mutex or vice versa, even when both primitives
    // have the same thread as owner.
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
    // Owner acquires both.
    host.dispatch(
        Lv2Request::LwMutexLock {
            id: lw_id,
            timeout: 0,
        },
        owner_unit,
        &rt,
    );
    host.dispatch(
        Lv2Request::MutexLock {
            mutex_id: hv_id,
            timeout: 0,
        },
        owner_unit,
        &rt,
    );
    // Waiter parks on the lwmutex only.
    host.dispatch(
        Lv2Request::LwMutexLock {
            id: lw_id,
            timeout: 0,
        },
        waiter_unit,
        &rt,
    );
    // lwmutex waiter list has waiter_tid; heavy mutex list is
    // empty.
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
    // Releasing the heavy mutex must not wake the lwmutex
    // waiter.
    let r = host.dispatch(Lv2Request::MutexUnlock { mutex_id: hv_id }, owner_unit, &rt);
    assert!(matches!(
        r,
        Lv2Dispatch::Immediate {
            code: 0,
            effects: _,
        }
    ));
    // lwmutex waiter still parked.
    assert_eq!(
        host.lwmutexes()
            .lookup(lw_id)
            .unwrap()
            .waiters()
            .iter()
            .collect::<Vec<_>>(),
        vec![waiter_tid],
    );
    // Releasing the lwmutex wakes the waiter.
    let r = host.dispatch(Lv2Request::LwMutexUnlock { id: lw_id }, owner_unit, &rt);
    match r {
        Lv2Dispatch::WakeAndReturn { woken_unit_ids, .. } => {
            assert_eq!(woken_unit_ids, vec![waiter_unit]);
        }
        other => panic!("expected WakeAndReturn, got {other:?}"),
    }
}

// ---------------------------------------------------------------
// Multi-primitive determinism canary.
//
// Two identical Lv2Host instances fed the same syscall sequence
// -- spanning PPU thread creation, heavy mutex lock/unlock,
// lwmutex lock/unlock, and semaphore wait/post cycles -- must
// produce byte-identical state hashes and byte-identical
// dispatch-outcome tags at every step. This is the guard
// against ordering nondeterminism: any such regression must
// trip this test before it ever reaches a real title.
// ---------------------------------------------------------------

#[test]
fn multi_primitive_determinism_canary() {
    fn canonical_run() -> Vec<(String, u64)> {
        let mut host = Lv2Host::new();
        let rt = FakeRuntime::new(0x10000);
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
                attr_ptr: 0,
                initial: 0,
                max: 4,
            },
            u0,
            &rt,
        ) {
            Lv2Dispatch::Immediate { effects: e, .. } => extract_write_u32(&e[0]),
            other => panic!("unexpected {other:?}"),
        };

        // Fixed syscall script. Each entry is (label, unit,
        // request). The label travels into the trace so test
        // output identifies which step first diverged if the
        // canary ever fails.
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
                    timeout: 0,
                },
            ),
            (
                "t1-lw-lock",
                u1,
                Lv2Request::LwMutexLock {
                    id: lwmutex_id,
                    timeout: 0,
                },
            ),
            (
                "t2-lw-lock",
                u2,
                Lv2Request::LwMutexLock {
                    id: lwmutex_id,
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
            // Classify the dispatch outcome as a short tag.
            // Payload details (effect vectors, specific woken
            // ids) are intentionally excluded: the canary
            // guards scheduler selection order, which the tag
            // plus the post-dispatch state hash together
            // capture.
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
    // Script covers lock/unlock/wait/post cycles on 3 distinct
    // PPU threads; a run with an empty script would trivially
    // pass. Guard against regression by asserting the script
    // actually ran non-empty state changes.
    assert!(run_a.len() >= 15);
}

// ---------------------------------------------------------------
// Lost-wake regression tests.
//
// For each primitive in the "release is remembered" family
// (lwmutex, mutex, semaphore, event queue, event flag), the
// release scheduled BEFORE the wait must observably unblock the
// would-be waiter. Test shape: run the release first on an empty
// primitive, then run the wait; the wait must complete Immediate,
// not Block. A handler that split the check-and-mutate across
// commit boundaries would park the waiter even though the
// release already landed -- a classic lost-wake bug.
//
// Cond is NOT in this family. A cond signal-before-wait is
// observably lost (covered by
// cond_signal_before_wait_does_not_wake_subsequent_waiter in
// src/host/cond.rs).
// ---------------------------------------------------------------

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
    host.dispatch(Lv2Request::LwMutexLock { id, timeout: 0 }, owner_unit, &rt);
    let unlock = host.dispatch(Lv2Request::LwMutexUnlock { id }, owner_unit, &rt);
    assert!(matches!(
        unlock,
        Lv2Dispatch::Immediate {
            code: 0,
            effects: _,
        }
    ));
    let lock = host.dispatch(Lv2Request::LwMutexLock { id, timeout: 0 }, later_unit, &rt);
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
    let rt = FakeRuntime::new(0x10000);
    let src = UnitId::new(0);
    seed_primary_ppu(&mut host, src);
    let created = host.dispatch(
        Lv2Request::SemaphoreCreate {
            id_ptr: 0x100,
            attr_ptr: 0,
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
    let rt = FakeRuntime::new(0x10000);
    let src = UnitId::new(0);
    seed_primary_ppu(&mut host, src);
    let created = host.dispatch(
        Lv2Request::EventFlagCreate {
            id_ptr: 0x100,
            attr_ptr: 0,
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
