//! LV2 dispatch for event queues.
//!
//! Waiters are FIFO. Receive parks with `payload = None`; send
//! installs `Some(payload)` through response_updates at wake time.
//! The wake path panics on `None` so a missing update surfaces
//! rather than delivering four zero u64s.

use cellgov_effects::{Effect, WritePayload};
use cellgov_event::{PriorityClass, UnitId};
use cellgov_mem::{ByteRange, GuestAddr};
use cellgov_ps3_abi::cell_errors;

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
        // size == 0 defaults to EQUEUE_MAX_RECV_EVENT (127).
        let effective_size = if size == 0 { 127 } else { size };
        let id = self.alloc_id();
        if !self.event_queues.create_with_id(id, effective_size) {
            return Lv2Dispatch::immediate(cell_errors::CELL_ENOMEM.into());
        }
        self.immediate_write_u32(id, id_ptr, requester)
    }

    pub(super) fn dispatch_event_queue_destroy(&mut self, id: u32) -> Lv2Dispatch {
        let Some(entry) = self.event_queues.lookup(id) else {
            return Lv2Dispatch::immediate(cell_errors::CELL_ESRCH.into());
        };
        if !entry.waiters().is_empty() {
            return Lv2Dispatch::immediate(cell_errors::CELL_EBUSY.into());
        }
        self.event_queues.destroy(id);
        Lv2Dispatch::immediate(0)
    }

    pub(super) fn dispatch_event_queue_receive(
        &mut self,
        id: u32,
        out_ptr: u32,
        requester: UnitId,
    ) -> Lv2Dispatch {
        let Some(caller) = self.ppu_threads.thread_id_for_unit(requester) else {
            return Lv2Dispatch::immediate(cell_errors::CELL_ESRCH.into());
        };
        match self.event_queues.try_receive(id) {
            None => Lv2Dispatch::immediate(cell_errors::CELL_ESRCH.into()),
            Some(crate::sync_primitives::EventQueueReceive::Delivered(payload)) => {
                // sys_event_t: four big-endian u64s at out_ptr.
                let mut buf = [0u8; 32];
                buf[0..8].copy_from_slice(&payload.source.to_be_bytes());
                buf[8..16].copy_from_slice(&payload.data1.to_be_bytes());
                buf[16..24].copy_from_slice(&payload.data2.to_be_bytes());
                buf[24..32].copy_from_slice(&payload.data3.to_be_bytes());
                let write = Effect::SharedWriteIntent {
                    range: ByteRange::contiguous_u32(out_ptr, 32),
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
                        return Lv2Dispatch::immediate(cell_errors::CELL_ESRCH.into());
                    }
                    Err(crate::sync_primitives::EventQueueEnqueueError::DuplicateWaiter) => {
                        return Lv2Dispatch::immediate(cell_errors::CELL_EFAULT.into());
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
        // try_receive_batch drains destructively; every output
        // ByteRange is validated up front.
        if ByteRange::new(GuestAddr::new(count_out as u64), 4).is_none() {
            return Lv2Dispatch::immediate(cell_errors::CELL_EFAULT.into());
        }
        for i in 0..size as u64 {
            let addr = event_array as u64 + i * 32;
            if ByteRange::new(GuestAddr::new(addr), 32).is_none() {
                return Lv2Dispatch::immediate(cell_errors::CELL_EFAULT.into());
            }
        }
        let Some(batch) = self.event_queues.try_receive_batch(id, size as usize) else {
            return Lv2Dispatch::immediate(cell_errors::CELL_ESRCH.into());
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
        let count_range = ByteRange::contiguous_u32(count_out, 4);
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
        // port_id == queue_id (1:1 binding).
        let payload = EventPayload {
            source: port_id as u64,
            data1,
            data2,
            data3,
        };
        match self.event_queues.send_and_wake_or_enqueue(port_id, payload) {
            crate::sync_primitives::EventQueueSend::Unknown => {
                Lv2Dispatch::immediate(cell_errors::CELL_ESRCH.into())
            }
            crate::sync_primitives::EventQueueSend::Full => {
                Lv2Dispatch::immediate(cell_errors::CELL_EBUSY.into())
            }
            crate::sync_primitives::EventQueueSend::Enqueued => Lv2Dispatch::immediate(0),
            crate::sync_primitives::EventQueueSend::Woke {
                new_owner,
                out_ptr,
                payload,
            } => match self.resolve_wake_thread(new_owner, "event_port_send.Woke") {
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
                None => Lv2Dispatch::immediate(0),
            },
        }
    }
}

#[cfg(test)]
#[path = "tests/event_queue_tests.rs"]
mod tests;
