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
use crate::sync_primitives::{EventPayload, SYS_EVENT_PORT_IPC, SYS_EVENT_PORT_LOCAL};

impl Lv2Host {
    /// A non-zero `ipc_key` registers the queue under that key so
    /// `sys_event_port_connect_ipc` (140) can find it.
    ///
    /// The oracle passes `SYS_SYNC_NEWLY_CREATED` unconditionally
    /// (RPCS3 `sys_event.cpp:251`), so unlike the shm path there is no
    /// resolve-or-create: a key already registered is `CELL_EEXIST`,
    /// and the way a second referent reaches the queue is 140, not a
    /// second create.
    ///
    /// # Errors
    ///
    /// - `CELL_ENOMEM` when the queue table rejects the id.
    /// - `CELL_EEXIST` when `ipc_key` is already registered.
    pub(super) fn dispatch_event_queue_create(
        &mut self,
        id_ptr: u32,
        ipc_key: u64,
        size: u32,
        requester: UnitId,
    ) -> Lv2Dispatch {
        if ipc_key != 0 && self.event_queue_ipc.contains_key(&ipc_key) {
            if super::is_system_ipc_key(ipc_key) {
                self.system_ipc_witness.event_queue_references += 1;
                self.system_ipc_witness.note_key(ipc_key);
            }
            return Lv2Dispatch::immediate(cell_errors::CELL_EEXIST.into());
        }
        // size == 0 defaults to EQUEUE_MAX_RECV_EVENT (127).
        let effective_size = if size == 0 { 127 } else { size };
        let id = self.alloc_id();
        if !self.event_queues.create_with_id(id, effective_size) {
            return Lv2Dispatch::immediate(cell_errors::CELL_ENOMEM.into());
        }
        if ipc_key != 0 {
            self.event_queue_ipc_keys.insert(id, ipc_key);
            self.event_queue_ipc.insert(ipc_key, id);
            if super::is_system_ipc_key(ipc_key) {
                self.system_ipc_witness.event_queue_creates += 1;
                self.system_ipc_witness.note_key(ipc_key);
            }
        }
        self.immediate_write_u32(id, id_ptr, requester)
    }

    /// `sys_event_port_create` (134).
    ///
    /// # Errors
    ///
    /// `CELL_EINVAL` for a `port_type` outside
    /// {`SYS_EVENT_PORT_LOCAL`, `SYS_EVENT_PORT_IPC`}. Oracle: RPCS3
    /// `sys_event.cpp:621`.
    pub(super) fn dispatch_event_port_create(
        &mut self,
        id_ptr: u32,
        port_type: u64,
        name: u64,
        requester: UnitId,
    ) -> Lv2Dispatch {
        if port_type != SYS_EVENT_PORT_LOCAL && port_type != SYS_EVENT_PORT_IPC {
            // The oracle logs this too (`sys_event.cpp:623`); a guest
            // that trips it is passing a type LV2 does not define.
            self.log_invariant_break(
                "dispatch.event_port_create_bad_type",
                format_args!("sys_event_port_create: port_type {port_type} is neither LOCAL(1) nor IPC(3); returning CELL_EINVAL"),
            );
            return Lv2Dispatch::immediate(cell_errors::CELL_EINVAL.into());
        }
        self.process_counts.event_port_inc();
        let id = self.alloc_id();
        self.event_ports.create_with_id(id, port_type, name);
        self.immediate_write_u32(id, id_ptr, requester)
    }

    /// `sys_event_port_destroy` (135).
    ///
    /// # Errors
    ///
    /// - `CELL_ESRCH` for an unknown port.
    /// - `CELL_EISCONN` while the port is still connected.
    pub(super) fn dispatch_event_port_destroy(&mut self, id: u32) -> Lv2Dispatch {
        use crate::sync_primitives::EventPortDestroyError as E;
        match self.event_ports.destroy(id) {
            Ok(()) => {
                self.process_counts.event_port_dec();
                Lv2Dispatch::immediate(0)
            }
            Err(E::UnknownPort) => Lv2Dispatch::immediate(cell_errors::CELL_ESRCH.into()),
            Err(E::Connected) => Lv2Dispatch::immediate(cell_errors::CELL_EISCONN.into()),
        }
    }

    /// `sys_event_port_connect_local` (136) and
    /// `sys_event_port_connect_ipc` (140).
    ///
    /// The two forms differ only in how the queue is named and which
    /// port type they accept, so they share one body. Oracle: RPCS3
    /// `sys_event.cpp:665-732`.
    ///
    /// # Errors
    ///
    /// - `CELL_EINVAL` when 140 is given `ipc_key == 0`, or the port's
    ///   type does not match the connect form.
    /// - `CELL_ESRCH` when either the port or the queue is absent.
    /// - `CELL_EISCONN` when the port already has a binding.
    fn dispatch_event_port_connect(
        &mut self,
        port_id: u32,
        queue_id: Option<u32>,
        required_type: u64,
    ) -> Lv2Dispatch {
        use crate::sync_primitives::EventPortConnectError as E;
        let Some(queue_id) = queue_id else {
            return Lv2Dispatch::immediate(cell_errors::CELL_ESRCH.into());
        };
        if self.event_queues.lookup(queue_id).is_none() {
            return Lv2Dispatch::immediate(cell_errors::CELL_ESRCH.into());
        }
        match self.event_ports.connect(port_id, queue_id, required_type) {
            Ok(()) => Lv2Dispatch::immediate(0),
            Err(E::UnknownPort) => Lv2Dispatch::immediate(cell_errors::CELL_ESRCH.into()),
            Err(E::WrongType) => Lv2Dispatch::immediate(cell_errors::CELL_EINVAL.into()),
            Err(E::AlreadyConnected) => Lv2Dispatch::immediate(cell_errors::CELL_EISCONN.into()),
        }
    }

    pub(super) fn dispatch_event_port_connect_local(
        &mut self,
        port_id: u32,
        queue_id: u32,
    ) -> Lv2Dispatch {
        self.dispatch_event_port_connect(port_id, Some(queue_id), SYS_EVENT_PORT_LOCAL)
    }

    /// Resolves `ipc_key` through the keyed-queue registry a keyed
    /// `sys_event_queue_create` populated.
    pub(super) fn dispatch_event_port_connect_ipc(
        &mut self,
        port_id: u32,
        ipc_key: u64,
    ) -> Lv2Dispatch {
        if ipc_key == 0 {
            return Lv2Dispatch::immediate(cell_errors::CELL_EINVAL.into());
        }
        let queue_id = self.event_queue_ipc.get(&ipc_key).copied();
        self.event_port_ipc_connects.0 += 1;
        if queue_id.is_some() {
            self.event_port_ipc_connects.1 += 1;
        }
        if super::is_system_ipc_key(ipc_key) {
            self.system_ipc_witness.event_port_connects += 1;
            self.system_ipc_witness.note_key(ipc_key);
        }
        self.dispatch_event_port_connect(port_id, queue_id, SYS_EVENT_PORT_IPC)
    }

    /// `sys_event_port_disconnect` (137).
    ///
    /// # Errors
    ///
    /// - `CELL_ESRCH` for an unknown port.
    /// - `CELL_ENOTCONN` when the port has no binding.
    pub(super) fn dispatch_event_port_disconnect(&mut self, port_id: u32) -> Lv2Dispatch {
        use crate::sync_primitives::EventPortDisconnectError as E;
        match self.event_ports.disconnect(port_id) {
            Ok(()) => Lv2Dispatch::immediate(0),
            Err(E::UnknownPort) => Lv2Dispatch::immediate(cell_errors::CELL_ESRCH.into()),
            Err(E::NotConnected) => Lv2Dispatch::immediate(cell_errors::CELL_ENOTCONN.into()),
        }
    }

    pub(super) fn dispatch_event_queue_destroy(&mut self, id: u32) -> Lv2Dispatch {
        let Some(entry) = self.event_queues.lookup(id) else {
            return Lv2Dispatch::immediate(cell_errors::CELL_ESRCH.into());
        };
        if !entry.waiters().is_empty() {
            return Lv2Dispatch::immediate(cell_errors::CELL_EBUSY.into());
        }
        self.event_queues.destroy(id);
        self.event_ports.unbind_queue(id);
        if let Some(ipc_key) = self.event_queue_ipc_keys.remove(&id) {
            if self.event_queue_ipc.get(&ipc_key) == Some(&id) {
                self.event_queue_ipc.remove(&ipc_key);
            }
        }
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
        // The event's `source` is the port, but delivery goes to the
        // queue the port is connected to.
        let Some(port) = self.event_ports.lookup(port_id) else {
            return Lv2Dispatch::immediate(cell_errors::CELL_ESRCH.into());
        };
        let Some(queue_id) = port.queue() else {
            return Lv2Dispatch::immediate(cell_errors::CELL_ENOTCONN.into());
        };
        let payload = EventPayload {
            source: port_id as u64,
            data1,
            data2,
            data3,
        };
        let sent = self
            .event_queues
            .send_and_wake_or_enqueue(queue_id, payload);
        // A refused send (unknown queue, full queue) is not a delivery.
        if matches!(
            sent,
            crate::sync_primitives::EventQueueSend::Enqueued
                | crate::sync_primitives::EventQueueSend::Woke { .. }
        ) {
            if let Some(&ipc_key) = self.event_queue_ipc_keys.get(&queue_id) {
                if super::is_system_ipc_key(ipc_key) {
                    self.system_ipc_witness.event_queue_enqueues += 1;
                    self.system_ipc_witness.note_key(ipc_key);
                }
            }
        }
        match sent {
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
