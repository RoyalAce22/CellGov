//! The `sys_config_*` syscalls, and the deliver, notify and complete steps they share.

use cellgov_effects::{Effect, WritePayload};
use cellgov_event::UnitId;
use cellgov_mem::ByteRange;
use cellgov_ps3_abi::lv2::config::{
    SYS_CONFIG_PADMANAGER_DS3_DESCRIPTOR, SYS_CONFIG_SERVICE_PADMANAGER,
    SYS_CONFIG_SERVICE_PADMANAGER2,
};
use cellgov_ps3_abi::lv2::errno;
use cellgov_time::GuestTicks;

use crate::dispatch::{Lv2Dispatch, PendingResponse};
use crate::host::{Lv2Host, Lv2Runtime};
use crate::sync_primitives::{EventPayload, EventQueueSend};

use super::table::{ConfigListener, ListenerSpec, ServiceSpec, SYS_CONFIG_DATA_CAP};

/// A parked receiver handed a service event during one dispatch.
struct ConfigWake {
    unit: UnitId,
    out_ptr: u32,
    payload: EventPayload,
}

impl Lv2Host {
    /// `sys_config_open` (516).
    ///
    /// The first open registers the two pad-manager services with the
    /// synthetic DUALSHOCK 3 descriptor.
    ///
    /// # Errors
    ///
    /// - `CELL_ESRCH` for an unknown event queue.
    /// - `CELL_EFAULT` for a null out-pointer.
    pub(in crate::host) fn dispatch_config_open(
        &mut self,
        equeue_id: u32,
        out_handle_ptr: u32,
        requester: UnitId,
        tick: GuestTicks,
    ) -> Lv2Dispatch {
        if self.state.event_queues.lookup(equeue_id).is_none() {
            return Lv2Dispatch::immediate(errno::CELL_ESRCH.into());
        }
        if out_handle_ptr == 0 {
            return Lv2Dispatch::immediate(errno::CELL_EFAULT.into());
        }
        if !self.state.config.seeded {
            for service_id in [
                SYS_CONFIG_SERVICE_PADMANAGER,
                SYS_CONFIG_SERVICE_PADMANAGER2,
            ] {
                let id = self.alloc_id();
                self.state.config.insert_service(
                    id,
                    ServiceSpec {
                        service_id,
                        user_id: 0,
                        verbosity: 1,
                        data_ptr: 0,
                        size: SYS_CONFIG_PADMANAGER_DS3_DESCRIPTOR.len() as u64,
                    },
                    SYS_CONFIG_PADMANAGER_DS3_DESCRIPTOR.to_vec(),
                );
            }
            self.state.config.seeded = true;
        }
        let id = self.alloc_id();
        self.state.config.insert_handle(id, equeue_id);
        self.immediate_write_u32(id, out_handle_ptr, requester, tick)
    }

    /// `sys_config_close` (517). Listeners outlive the handle and keep
    /// delivering to its queue; only record reads through the closed
    /// handle stop.
    ///
    /// # Errors
    ///
    /// `CELL_ESRCH` for an unknown handle.
    pub(in crate::host) fn dispatch_config_close(&mut self, handle: u32) -> Lv2Dispatch {
        if self.state.config.remove_handle(handle) {
            Lv2Dispatch::immediate(0)
        } else {
            Lv2Dispatch::immediate(errno::CELL_ESRCH.into())
        }
    }

    /// `sys_config_get_service_event` (518): writes the
    /// `sys_config_service_event_t` record behind a queued event. The
    /// record's `registered` field and its length follow the service's
    /// state at read time, whatever the queued event announced.
    ///
    /// # Errors
    ///
    /// - `CELL_ESRCH` for an unknown handle, an unknown event, or an
    ///   event sent through a different handle.
    /// - `CELL_EAGAIN` when `size` is below the length the queued
    ///   event's `data3` announced, which exceeds the bytes written
    ///   (see [`SYS_CONFIG_SERVICE_EVENT_ANNOUNCED_HEAD_LEN`](cellgov_ps3_abi::lv2::config::SYS_CONFIG_SERVICE_EVENT_ANNOUNCED_HEAD_LEN)).
    /// - `CELL_EFAULT` for a null destination.
    pub(in crate::host) fn dispatch_config_get_service_event(
        &mut self,
        handle: u32,
        event_id: u32,
        dst_ptr: u32,
        size: u64,
        requester: UnitId,
        tick: GuestTicks,
    ) -> Lv2Dispatch {
        if self.state.config.handle(handle).is_none() {
            return Lv2Dispatch::immediate(errno::CELL_ESRCH.into());
        }
        let Some(event) = self.state.config.event(event_id) else {
            return Lv2Dispatch::immediate(errno::CELL_ESRCH.into());
        };
        if event.handle != handle {
            return Lv2Dispatch::immediate(errno::CELL_ESRCH.into());
        }
        let Some(service) = self.state.config.service(event.service) else {
            return Lv2Dispatch::immediate(errno::CELL_ESRCH.into());
        };
        if size < service.announced_len() as u64 {
            return Lv2Dispatch::immediate(errno::CELL_EAGAIN.into());
        }
        let Some(record) = self.state.config.record(event_id) else {
            return Lv2Dispatch::immediate(errno::CELL_ESRCH.into());
        };
        if dst_ptr == 0 {
            return Lv2Dispatch::immediate(errno::CELL_EFAULT.into());
        }
        Lv2Dispatch::Immediate {
            code: 0,
            effects: vec![Effect::shared_write(
                ByteRange::contiguous_u32(dst_ptr, record.len() as u32),
                WritePayload::from_slice(&record),
                requester,
                tick,
            )],
        }
    }

    /// `sys_config_add_service_listener` (519): registers the listener
    /// and replays every matching service already registered, oldest
    /// first, to the handle's queue.
    ///
    /// # Errors
    ///
    /// - `CELL_ESRCH` for an unknown handle.
    /// - `CELL_EFAULT` for a null out-pointer or an unreadable data
    ///   buffer.
    /// - `CELL_EINVAL` when the buffer exceeds [`SYS_CONFIG_DATA_CAP`].
    pub(in crate::host) fn dispatch_config_add_service_listener(
        &mut self,
        handle: u32,
        spec: ListenerSpec,
        out_listener_ptr: u32,
        requester: UnitId,
        rt: &dyn Lv2Runtime,
        tick: GuestTicks,
    ) -> Lv2Dispatch {
        let Some(h) = self.state.config.handle(handle) else {
            return Lv2Dispatch::immediate(errno::CELL_ESRCH.into());
        };
        if out_listener_ptr == 0 {
            return Lv2Dispatch::immediate(errno::CELL_EFAULT.into());
        }
        let data = match self.config_read_data("add_service_listener", spec.in_ptr, spec.size, rt) {
            Ok(data) => data,
            Err(code) => return Lv2Dispatch::immediate(code.into()),
        };
        let id = self.alloc_id();
        self.state.config.insert_listener(
            id,
            ConfigListener {
                handle,
                queue_id: h.queue_id,
                service_id: spec.service_id,
                min_verbosity: spec.min_verbosity,
                listener_type: spec.listener_type,
                data,
                delivered: 0,
            },
        );
        let mut wakes = Vec::new();
        for service in self.state.config.matching_services(id) {
            wakes.extend(self.config_deliver(id, service));
        }
        self.config_complete(id, out_listener_ptr, wakes, requester, tick)
    }

    /// `sys_config_remove_service_listener` (520). The handle is not
    /// consulted; the listener's undelivered records go with it.
    ///
    /// # Errors
    ///
    /// `CELL_ESRCH` for an unknown listener.
    pub(in crate::host) fn dispatch_config_remove_service_listener(
        &mut self,
        _handle: u32,
        listener: u32,
    ) -> Lv2Dispatch {
        if self.state.config.remove_listener(listener) {
            Lv2Dispatch::immediate(0)
        } else {
            Lv2Dispatch::immediate(errno::CELL_ESRCH.into())
        }
    }

    /// `sys_config_register_service` (521): registers a service and
    /// notifies every listener that matches it.
    ///
    /// # Errors
    ///
    /// - `CELL_ESRCH` for an unknown handle.
    /// - `CELL_EFAULT` for a null out-pointer or an unreadable data
    ///   buffer.
    /// - `CELL_EINVAL` when the buffer exceeds [`SYS_CONFIG_DATA_CAP`].
    pub(in crate::host) fn dispatch_config_register_service(
        &mut self,
        handle: u32,
        spec: ServiceSpec,
        out_service_ptr: u32,
        requester: UnitId,
        rt: &dyn Lv2Runtime,
        tick: GuestTicks,
    ) -> Lv2Dispatch {
        if self.state.config.handle(handle).is_none() {
            return Lv2Dispatch::immediate(errno::CELL_ESRCH.into());
        }
        if out_service_ptr == 0 {
            return Lv2Dispatch::immediate(errno::CELL_EFAULT.into());
        }
        let data = match self.config_read_data("register_service", spec.data_ptr, spec.size, rt) {
            Ok(data) => data,
            Err(code) => return Lv2Dispatch::immediate(code.into()),
        };
        let id = self.alloc_id();
        self.state.config.insert_service(id, spec, data);
        let wakes = self.config_notify_listeners(id);
        self.config_complete(id, out_service_ptr, wakes, requester, tick)
    }

    /// `sys_config_unregister_service` (522): flips the service to
    /// unregistered and notifies matching listeners with a
    /// `registered = 0` event. The handle is not consulted.
    ///
    /// # Errors
    ///
    /// `CELL_ESRCH` for an unknown or already-unregistered service.
    pub(in crate::host) fn dispatch_config_unregister_service(
        &mut self,
        _handle: u32,
        service: u32,
    ) -> Lv2Dispatch {
        if !self.state.config.unregister(service) {
            return Lv2Dispatch::immediate(errno::CELL_ESRCH.into());
        }
        let wakes = self.config_notify_listeners(service);
        self.state.config.collect_service(service);
        self.config_finish(0, wakes, vec![])
    }

    /// Read a listener or service data buffer; `Err` carries the
    /// `CELL_*` code the arm returns.
    fn config_read_data(
        &mut self,
        arm: &str,
        ptr: u32,
        size: u64,
        rt: &dyn Lv2Runtime,
    ) -> Result<Vec<u8>, errno::Lv2ErrCode> {
        if size == 0 {
            return Ok(Vec::new());
        }
        if size > SYS_CONFIG_DATA_CAP {
            self.log_invariant_break(
                "dispatch.config_data_over_cap",
                format_args!(
                    "sys_config_{arm}: data buffer of {size} bytes exceeds the {SYS_CONFIG_DATA_CAP}-byte cap; returning CELL_EINVAL"
                ),
            );
            return Err(errno::CELL_EINVAL);
        }
        match rt.read_committed(u64::from(ptr), size as usize) {
            Some(bytes) => Ok(bytes.to_vec()),
            None => Err(errno::CELL_EFAULT),
        }
    }

    /// Send one event for `(listener, service)`; `Some` when the send
    /// handed the payload to a parked receiver.
    fn config_deliver(&mut self, listener: u32, service: u32) -> Option<ConfigWake> {
        let (event, queue_id, payload) = self.state.config.stage_event(listener, service)?;
        match self
            .state
            .event_queues
            .send_and_wake_or_enqueue(queue_id, payload)
        {
            EventQueueSend::Enqueued => None,
            EventQueueSend::Woke {
                new_owner,
                out_ptr,
                payload,
            } => self
                .resolve_wake_thread(new_owner, "config_deliver.Woke")
                .map(|unit| ConfigWake {
                    unit,
                    out_ptr,
                    payload,
                }),
            // A refused send is not a delivery: the event is dropped
            // rather than leaving a record no queue announced.
            EventQueueSend::Unknown | EventQueueSend::Full => {
                self.obs.config_events_dropped += 1;
                self.state.config.discard_event(event);
                None
            }
        }
    }

    fn config_notify_listeners(&mut self, service: u32) -> Vec<ConfigWake> {
        let mut wakes = Vec::new();
        for listener in self.state.config.matching_listeners(service) {
            wakes.extend(self.config_deliver(listener, service));
        }
        wakes
    }

    /// Write the minted `id` to `out_ptr` and resolve any receivers
    /// the deliveries woke.
    fn config_complete(
        &mut self,
        id: u32,
        out_ptr: u32,
        wakes: Vec<ConfigWake>,
        requester: UnitId,
        tick: GuestTicks,
    ) -> Lv2Dispatch {
        let write = Effect::shared_write(
            ByteRange::contiguous_u32(out_ptr, 4),
            WritePayload::from_slice(&id.to_be_bytes()),
            requester,
            tick,
        );
        self.config_finish(0, wakes, vec![write])
    }

    fn config_finish(
        &self,
        code: u64,
        wakes: Vec<ConfigWake>,
        effects: Vec<Effect>,
    ) -> Lv2Dispatch {
        if wakes.is_empty() {
            return Lv2Dispatch::Immediate { code, effects };
        }
        let woken_unit_ids = wakes.iter().map(|w| w.unit).collect();
        let response_updates = wakes
            .into_iter()
            .map(|w| {
                (
                    w.unit,
                    PendingResponse::EventQueueReceive {
                        out_ptr: w.out_ptr,
                        payload: Some(w.payload),
                    },
                )
            })
            .collect();
        Lv2Dispatch::WakeAndReturn {
            code,
            woken_unit_ids,
            response_updates,
            effects,
        }
    }
}
