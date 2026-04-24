//! LV2 dispatch for event queues.
//!
//! Waiters are FIFO. Receive parks with `payload = None`; the
//! send-side dispatch installs `Some(payload)` through
//! response_updates at wake time. The wake path panics on `None`
//! so a missing update surfaces rather than delivering four zero
//! u64s indistinguishable from a real event.

use cellgov_effects::{Effect, WritePayload};
use cellgov_event::{PriorityClass, UnitId};
use cellgov_mem::{ByteRange, GuestAddr};

use crate::dispatch::{Lv2Dispatch, PendingResponse};
use crate::host::Lv2Host;
use crate::sync_primitives::EventPayload;

impl Lv2Host {
    pub(super) fn dispatch_event_queue_create(
        &mut self,
        id_ptr: u32,
        size: u32,
        requester: UnitId,
    ) -> Lv2Dispatch {
        // size == 0 defaults to EQUEUE_MAX_RECV_EVENT (127) per
        // the permissive ABI.
        let effective_size = if size == 0 { 127 } else { size };
        let id = self.alloc_id();
        if !self.event_queues.create_with_id(id, effective_size) {
            return Lv2Dispatch::Immediate {
                code: crate::errno::CELL_ENOMEM.into(),
                effects: vec![],
            };
        }
        self.immediate_write_u32(id, id_ptr, requester)
    }

    pub(super) fn dispatch_event_queue_destroy(&mut self, id: u32) -> Lv2Dispatch {
        let Some(entry) = self.event_queues.lookup(id) else {
            return Lv2Dispatch::Immediate {
                code: crate::errno::CELL_ESRCH.into(),
                effects: vec![],
            };
        };
        if !entry.waiters().is_empty() {
            return Lv2Dispatch::Immediate {
                code: crate::errno::CELL_EBUSY.into(),
                effects: vec![],
            };
        }
        self.event_queues.destroy(id);
        Lv2Dispatch::Immediate {
            code: 0,
            effects: vec![],
        }
    }

    pub(super) fn dispatch_event_queue_receive(
        &mut self,
        id: u32,
        out_ptr: u32,
        requester: UnitId,
    ) -> Lv2Dispatch {
        let Some(caller) = self.ppu_threads.thread_id_for_unit(requester) else {
            return Lv2Dispatch::Immediate {
                code: crate::errno::CELL_ESRCH.into(),
                effects: vec![],
            };
        };
        match self.event_queues.try_receive(id) {
            None => Lv2Dispatch::Immediate {
                code: crate::errno::CELL_ESRCH.into(),
                effects: vec![],
            },
            Some(crate::sync_primitives::EventQueueReceive::Delivered(payload)) => {
                // sys_event_t: four big-endian u64s at out_ptr.
                let mut buf = [0u8; 32];
                buf[0..8].copy_from_slice(&payload.source.to_be_bytes());
                buf[8..16].copy_from_slice(&payload.data1.to_be_bytes());
                buf[16..24].copy_from_slice(&payload.data2.to_be_bytes());
                buf[24..32].copy_from_slice(&payload.data3.to_be_bytes());
                let write = Effect::SharedWriteIntent {
                    range: ByteRange::new(GuestAddr::new(out_ptr as u64), 32).unwrap(),
                    bytes: WritePayload::from_slice(&buf),
                    ordering: PriorityClass::Normal,
                    source: requester,
                    source_time: self.current_tick,
                };
                Lv2Dispatch::Immediate {
                    code: 0,
                    effects: vec![write],
                }
            }
            Some(crate::sync_primitives::EventQueueReceive::Empty) => {
                match self.event_queues.enqueue_waiter(id, caller, out_ptr) {
                    Ok(()) => {}
                    Err(crate::sync_primitives::EventQueueEnqueueError::UnknownId) => {
                        return Lv2Dispatch::Immediate {
                            code: crate::errno::CELL_ESRCH.into(),
                            effects: vec![],
                        };
                    }
                    Err(crate::sync_primitives::EventQueueEnqueueError::DuplicateWaiter) => {
                        return Lv2Dispatch::Immediate {
                            code: crate::errno::CELL_EFAULT.into(),
                            effects: vec![],
                        };
                    }
                }
                Lv2Dispatch::Block {
                    reason: crate::dispatch::Lv2BlockReason::EventQueue { id },
                    pending: PendingResponse::EventQueueReceive {
                        out_ptr,
                        payload: None,
                    },
                    effects: vec![],
                }
            }
        }
    }

    pub(super) fn dispatch_event_queue_tryreceive(
        &mut self,
        id: u32,
        event_array: u32,
        size: u32,
        count_out: u32,
        requester: UnitId,
    ) -> Lv2Dispatch {
        // try_receive_batch drains destructively, so every
        // output ByteRange must be validated up front or a bad
        // address could silently discard events while count_out
        // still claims them.
        if ByteRange::new(GuestAddr::new(count_out as u64), 4).is_none() {
            return Lv2Dispatch::Immediate {
                code: crate::errno::CELL_EFAULT.into(),
                effects: vec![],
            };
        }
        for i in 0..size as u64 {
            let addr = event_array as u64 + i * 32;
            if ByteRange::new(GuestAddr::new(addr), 32).is_none() {
                return Lv2Dispatch::Immediate {
                    code: crate::errno::CELL_EFAULT.into(),
                    effects: vec![],
                };
            }
        }
        let Some(batch) = self.event_queues.try_receive_batch(id, size as usize) else {
            return Lv2Dispatch::Immediate {
                code: crate::errno::CELL_ESRCH.into(),
                effects: vec![],
            };
        };
        let count = batch.len() as u32;
        let mut effects: Vec<Effect> = Vec::with_capacity(batch.len() + 1);
        for (i, payload) in batch.iter().enumerate() {
            let mut buf = [0u8; 32];
            buf[0..8].copy_from_slice(&payload.source.to_be_bytes());
            buf[8..16].copy_from_slice(&payload.data1.to_be_bytes());
            buf[16..24].copy_from_slice(&payload.data2.to_be_bytes());
            buf[24..32].copy_from_slice(&payload.data3.to_be_bytes());
            let addr = event_array as u64 + (i as u64) * 32;
            let range = ByteRange::new(GuestAddr::new(addr), 32).expect("validated above");
            effects.push(Effect::SharedWriteIntent {
                range,
                bytes: WritePayload::from_slice(&buf),
                ordering: PriorityClass::Normal,
                source: requester,
                source_time: self.current_tick,
            });
        }
        let count_range =
            ByteRange::new(GuestAddr::new(count_out as u64), 4).expect("validated above");
        effects.push(Effect::SharedWriteIntent {
            range: count_range,
            bytes: WritePayload::from_slice(&count.to_be_bytes()),
            ordering: PriorityClass::Normal,
            source: requester,
            source_time: self.current_tick,
        });
        Lv2Dispatch::Immediate {
            code: 0u64,
            effects,
        }
    }

    pub(super) fn dispatch_event_port_send(
        &mut self,
        port_id: u32,
        data1: u64,
        data2: u64,
        data3: u64,
    ) -> Lv2Dispatch {
        // Port id == queue id (1:1 binding).
        let payload = EventPayload {
            source: port_id as u64,
            data1,
            data2,
            data3,
        };
        match self.event_queues.send_and_wake_or_enqueue(port_id, payload) {
            crate::sync_primitives::EventQueueSend::Unknown => Lv2Dispatch::Immediate {
                code: crate::errno::CELL_ESRCH.into(),
                effects: vec![],
            },
            crate::sync_primitives::EventQueueSend::Full => Lv2Dispatch::Immediate {
                code: crate::errno::CELL_EBUSY.into(),
                effects: vec![],
            },
            crate::sync_primitives::EventQueueSend::Enqueued => Lv2Dispatch::Immediate {
                code: 0,
                effects: vec![],
            },
            crate::sync_primitives::EventQueueSend::Woke {
                new_owner,
                out_ptr,
                payload,
            } => {
                // Missing thread entry is a host-invariant break;
                // payload is lost rather than delivered blind.
                match self.resolve_wake_thread(new_owner, "event_port_send.Woke") {
                    Some(unit) => Lv2Dispatch::WakeAndReturn {
                        code: 0,
                        woken_unit_ids: vec![unit],
                        response_updates: vec![(
                            unit,
                            PendingResponse::EventQueueReceive {
                                out_ptr,
                                payload: Some(payload),
                            },
                        )],
                        effects: vec![],
                    },
                    None => Lv2Dispatch::Immediate {
                        code: 0,
                        effects: vec![],
                    },
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::test_support::{extract_write_u32, seed_primary_ppu, FakeRuntime};
    use crate::ppu_thread::{PpuThreadAttrs, PpuThreadId};
    use crate::request::Lv2Request;

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
        assert_eq!(code, crate::errno::CELL_EBUSY.into());
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
            Lv2Dispatch::Immediate {
                code: 0,
                effects: e,
            } => match &e[0] {
                Effect::SharedWriteIntent { range, bytes, .. } => {
                    assert_eq!(range.start().raw(), 0x2000);
                    assert_eq!(range.length(), 32);
                    let payload_bytes = bytes.bytes();
                    assert_eq!(
                        u64::from_be_bytes(payload_bytes[0..8].try_into().unwrap()),
                        0x11
                    );
                    assert_eq!(
                        u64::from_be_bytes(payload_bytes[8..16].try_into().unwrap()),
                        0x22
                    );
                }
                other => panic!("expected SharedWriteIntent, got {other:?}"),
            },
            other => panic!("expected Immediate(0), got {other:?}"),
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
        let send = host.dispatch(
            Lv2Request::EventPortSend {
                port_id: id,
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
                        assert_eq!(p.source, id as u64);
                        assert_eq!(p.data1, 0xAA);
                        assert_eq!(p.data2, 0xBB);
                        assert_eq!(p.data3, 0xCC);
                    }
                    other => panic!("expected EventQueueReceive(Some(..)), got {other:?}"),
                }
            }
            other => panic!("expected WakeAndReturn, got {other:?}"),
        }
        // Fast-path handoff: queue storage unchanged, waiter list
        // drained.
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
                // Two payload writes plus the count_out write.
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
        assert_eq!(code, crate::errno::CELL_ESRCH.into());
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
        let send = host.dispatch(
            Lv2Request::EventPortSend {
                port_id: id,
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
}
