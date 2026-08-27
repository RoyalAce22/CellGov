//! LV2 event-queue and event-port dispatch tests: buffered payload delivery, receive parking, tryreceive batch draining, and EBUSY destroy.

use super::*;
use crate::host::test_support::{connected_port, extract_write_u32, seed_primary_ppu, FakeRuntime};
use crate::ppu_thread::{PpuThreadAttrs, PpuThreadId};
use crate::request::Lv2Request;

#[test]
fn a_receive_from_a_unit_without_a_thread_record_is_esrch_and_logged() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let breaks_before = host.observability().invariant_break_count;
    let d = host.dispatch(
        Lv2Request::EventQueueReceive {
            queue_id: 0x4000_0001,
            out_ptr: 0x2000,
            timeout: 0,
        },
        UnitId::new(9),
        &rt,
    );
    assert_eq!(
        d,
        Lv2Dispatch::immediate(u64::from(cellgov_ps3_abi::cell_errors::CELL_ESRCH))
    );
    assert!(host.observability().invariant_break_count > breaks_before);
}

#[test]
fn event_queue_create_allocates_id() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let result = host.dispatch(
        Lv2Request::EventQueueCreate {
            id_ptr: 0x100,
            attr_ptr: 0x200,
            key: 0,
            size: 64,
        },
        UnitId::new(0),
        &rt,
    );
    match result {
        Lv2Dispatch::Immediate { code: 0, effects } => {
            assert_eq!(effects.len(), 1);
            let id = extract_write_u32(&effects[0]);
            assert!(id > 0, "queue ID should be non-zero");
        }
        other => panic!("expected Immediate(0), got {other:?}"),
    }
}

#[test]
fn event_queue_create_writes_id_and_stores_entry() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let src = UnitId::new(0);
    seed_primary_ppu(&mut host, src);
    let r = host.dispatch(
        Lv2Request::EventQueueCreate {
            id_ptr: 0x100,
            attr_ptr: 0,
            key: 0,
            size: 8,
        },
        src,
        &rt,
    );
    let id = match &r {
        Lv2Dispatch::Immediate {
            code: 0,
            effects: e,
        } => extract_write_u32(&e[0]),
        other => panic!("expected Immediate(0), got {other:?}"),
    };
    assert_eq!(host.event_queues().lookup(id).unwrap().size(), 8);
}

#[test]
fn event_queue_destroy_with_waiters_returns_ebusy() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let src = UnitId::new(0);
    seed_primary_ppu(&mut host, src);
    let r = host.dispatch(
        Lv2Request::EventQueueCreate {
            id_ptr: 0x100,
            attr_ptr: 0,
            key: 0,
            size: 4,
        },
        src,
        &rt,
    );
    let id = match &r {
        Lv2Dispatch::Immediate {
            code: 0,
            effects: e,
        } => extract_write_u32(&e[0]),
        other => panic!("expected Immediate(0), got {other:?}"),
    };
    host.event_queues_mut()
        .enqueue_waiter(id, PpuThreadId::PRIMARY, 0x2000)
        .unwrap();
    let d = host.dispatch(Lv2Request::EventQueueDestroy { queue_id: id }, src, &rt);
    let Lv2Dispatch::Immediate { code, .. } = d else {
        panic!("expected Immediate, got {d:?}");
    };
    assert_eq!(code, cell_errors::CELL_EBUSY.into());
}

#[test]
fn event_queue_receive_with_buffered_payload_delivers_immediately() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let src = UnitId::new(0);
    seed_primary_ppu(&mut host, src);
    let r = host.dispatch(
        Lv2Request::EventQueueCreate {
            id_ptr: 0x100,
            attr_ptr: 0,
            key: 0,
            size: 4,
        },
        src,
        &rt,
    );
    let id = match &r {
        Lv2Dispatch::Immediate {
            code: 0,
            effects: e,
        } => extract_write_u32(&e[0]),
        other => panic!("expected Immediate(0), got {other:?}"),
    };
    host.event_queues_mut().send_and_wake_or_enqueue(
        id,
        crate::sync_primitives::EventPayload {
            source: 0x11,
            data1: 0x22,
            data2: 0x33,
            data3: 0x44,
        },
    );
    let recv = host.dispatch(
        Lv2Request::EventQueueReceive {
            queue_id: id,
            out_ptr: 0x2000,
            timeout: 0,
        },
        src,
        &rt,
    );
    match recv {
        Lv2Dispatch::ImmediateRegisters {
            code: 0,
            effects,
            registers,
        } => {
            // The event lands in r4..=r7; the out-pointer is never written.
            assert!(
                effects.is_empty(),
                "no guest memory is written: {effects:?}"
            );
            assert_eq!(registers[0], (4, 0x11));
            assert_eq!(registers[1], (5, 0x22));
            assert_eq!(registers.len(), 4);
        }
        other => panic!("expected ImmediateRegisters(0), got {other:?}"),
    }
    assert!(host.event_queues().lookup(id).unwrap().is_empty());
}

#[test]
fn event_queue_receive_empty_parks_caller() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let src = UnitId::new(0);
    seed_primary_ppu(&mut host, src);
    let r = host.dispatch(
        Lv2Request::EventQueueCreate {
            id_ptr: 0x100,
            attr_ptr: 0,
            key: 0,
            size: 4,
        },
        src,
        &rt,
    );
    let id = match &r {
        Lv2Dispatch::Immediate {
            code: 0,
            effects: e,
        } => extract_write_u32(&e[0]),
        other => panic!("expected Immediate(0), got {other:?}"),
    };
    let recv = host.dispatch(
        Lv2Request::EventQueueReceive {
            queue_id: id,
            out_ptr: 0x2000,
            timeout: 0,
        },
        src,
        &rt,
    );
    match recv {
        Lv2Dispatch::Block {
            reason: crate::dispatch::Lv2BlockReason::EventQueue { id: bid },
            pending: PendingResponse::EventQueueReceive { out_ptr, payload },
            ..
        } => {
            assert_eq!(bid, id);
            assert_eq!(out_ptr, 0x2000);
            assert_eq!(payload, None);
        }
        other => panic!("expected Block on EventQueue, got {other:?}"),
    }
    let waiters: Vec<_> = host
        .event_queues()
        .lookup(id)
        .unwrap()
        .waiters()
        .iter()
        .map(|w| (w.thread, w.out_ptr))
        .collect();
    assert_eq!(waiters, vec![(PpuThreadId::PRIMARY, 0x2000)]);
}

#[test]
fn event_port_send_with_parked_waiter_emits_wake_and_return_with_payload() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let sender_unit = UnitId::new(0);
    let waiter_unit = UnitId::new(1);
    seed_primary_ppu(&mut host, sender_unit);
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
    let r = host.dispatch(
        Lv2Request::EventQueueCreate {
            id_ptr: 0x100,
            attr_ptr: 0,
            key: 0,
            size: 4,
        },
        sender_unit,
        &rt,
    );
    let id = match &r {
        Lv2Dispatch::Immediate {
            code: 0,
            effects: e,
        } => extract_write_u32(&e[0]),
        other => panic!("expected Immediate(0), got {other:?}"),
    };
    host.dispatch(
        Lv2Request::EventQueueReceive {
            queue_id: id,
            out_ptr: 0x2000,
            timeout: 0,
        },
        waiter_unit,
        &rt,
    );
    let port_id = connected_port(&mut host, &rt, sender_unit, id);
    let send = host.dispatch(
        Lv2Request::EventPortSend {
            port_id,
            data1: 0xAA,
            data2: 0xBB,
            data3: 0xCC,
        },
        sender_unit,
        &rt,
    );
    match send {
        Lv2Dispatch::WakeAndReturn {
            code: 0,
            woken_unit_ids,
            response_updates,
            ..
        } => {
            assert_eq!(woken_unit_ids, vec![waiter_unit]);
            assert_eq!(response_updates.len(), 1);
            let (u, resp) = &response_updates[0];
            assert_eq!(*u, waiter_unit);
            match resp {
                PendingResponse::EventQueueReceive {
                    payload: Some(p), ..
                } => {
                    // `source` identifies the sending PORT, not the
                    // queue the event landed in.
                    assert_eq!(p.source, u64::from(port_id));
                    assert_eq!(p.data1, 0xAA);
                    assert_eq!(p.data2, 0xBB);
                    assert_eq!(p.data3, 0xCC);
                }
                other => panic!("expected EventQueueReceive(Some(..)), got {other:?}"),
            }
        }
        other => panic!("expected WakeAndReturn, got {other:?}"),
    }
    assert!(host.event_queues().lookup(id).unwrap().is_empty());
    assert!(host.event_queues().lookup(id).unwrap().waiters().is_empty());
}

#[test]
fn event_queue_tryreceive_batch_drains_payloads() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let src = UnitId::new(0);
    seed_primary_ppu(&mut host, src);
    let r = host.dispatch(
        Lv2Request::EventQueueCreate {
            id_ptr: 0x100,
            attr_ptr: 0,
            key: 0,
            size: 4,
        },
        src,
        &rt,
    );
    let id = match &r {
        Lv2Dispatch::Immediate {
            code: 0,
            effects: e,
        } => extract_write_u32(&e[0]),
        other => panic!("expected Immediate(0), got {other:?}"),
    };
    for i in 1..=3u64 {
        host.event_queues_mut().send_and_wake_or_enqueue(
            id,
            crate::sync_primitives::EventPayload {
                source: i,
                data1: i * 10,
                data2: 0,
                data3: 0,
            },
        );
    }
    let tr = host.dispatch(
        Lv2Request::EventQueueTryReceive {
            queue_id: id,
            event_array: 0x2000,
            size: 2,
            count_out: 0x3000,
        },
        src,
        &rt,
    );
    match tr {
        Lv2Dispatch::Immediate {
            code: 0,
            effects: e,
        } => {
            assert_eq!(e.len(), 3);
        }
        other => panic!("expected Immediate(0), got {other:?}"),
    }
    assert_eq!(host.event_queues().lookup(id).unwrap().len(), 1);
}

#[test]
fn event_queue_tryreceive_empty_writes_zero_count() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let src = UnitId::new(0);
    seed_primary_ppu(&mut host, src);
    let r = host.dispatch(
        Lv2Request::EventQueueCreate {
            id_ptr: 0x100,
            attr_ptr: 0,
            key: 0,
            size: 4,
        },
        src,
        &rt,
    );
    let id = match &r {
        Lv2Dispatch::Immediate {
            code: 0,
            effects: e,
        } => extract_write_u32(&e[0]),
        other => panic!("expected Immediate(0), got {other:?}"),
    };
    let tr = host.dispatch(
        Lv2Request::EventQueueTryReceive {
            queue_id: id,
            event_array: 0x2000,
            size: 2,
            count_out: 0x3000,
        },
        src,
        &rt,
    );
    match tr {
        Lv2Dispatch::Immediate {
            code: 0,
            effects: e,
        } => {
            assert_eq!(e.len(), 1);
        }
        other => panic!("expected Immediate(0), got {other:?}"),
    }
}

#[test]
fn event_queue_tryreceive_unknown_returns_esrch() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let r = host.dispatch(
        Lv2Request::EventQueueTryReceive {
            queue_id: 99,
            event_array: 0x2000,
            size: 2,
            count_out: 0x3000,
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
fn event_port_send_with_no_waiters_enqueues_payload() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let src = UnitId::new(0);
    seed_primary_ppu(&mut host, src);
    let r = host.dispatch(
        Lv2Request::EventQueueCreate {
            id_ptr: 0x100,
            attr_ptr: 0,
            key: 0,
            size: 4,
        },
        src,
        &rt,
    );
    let id = match &r {
        Lv2Dispatch::Immediate {
            code: 0,
            effects: e,
        } => extract_write_u32(&e[0]),
        other => panic!("expected Immediate(0), got {other:?}"),
    };
    let port_id = connected_port(&mut host, &rt, src, id);
    let send = host.dispatch(
        Lv2Request::EventPortSend {
            port_id,
            data1: 0xAA,
            data2: 0xBB,
            data3: 0xCC,
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
    assert_eq!(host.event_queues().lookup(id).unwrap().len(), 1);
}
