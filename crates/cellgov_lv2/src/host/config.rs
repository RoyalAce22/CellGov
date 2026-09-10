//! `sys_config` (516-522): the subscription store through which the
//! shell's device managers learn what is attached.
//!
//! A handle binds to an event queue. Listeners subscribe to one
//! service id; every registered service a listener matches is
//! replayed to the handle's queue as a service event, and the guest
//! reads the record behind an event with
//! `sys_config_get_service_event`. Ids for handles, listeners, and
//! services come from the shared kernel-id allocator. Event ids count
//! from zero, which is CellGov's own scheme; the kernel's own
//! numbering is guest-visible and nothing in dev_flash states it.

use std::collections::BTreeMap;

use cellgov_effects::{Effect, WritePayload};
use cellgov_event::{PriorityClass, UnitId};
use cellgov_mem::ByteRange;
use cellgov_ps3_abi::lv2::config::{
    SYS_CONFIG_EVENT_SOURCE_SERVICE, SYS_CONFIG_PADMANAGER_DS3_DESCRIPTOR,
    SYS_CONFIG_SERVICE_EVENT_ANNOUNCED_HEAD_LEN, SYS_CONFIG_SERVICE_EVENT_HEAD_LEN,
    SYS_CONFIG_SERVICE_EVENT_UNREGISTERED_LEN, SYS_CONFIG_SERVICE_LISTENER_ONCE,
    SYS_CONFIG_SERVICE_PADMANAGER, SYS_CONFIG_SERVICE_PADMANAGER2,
};
use cellgov_ps3_abi::lv2::errno;
use cellgov_time::GuestTicks;

use crate::dispatch::{Lv2Dispatch, PendingResponse};
use crate::host::{Lv2Host, Lv2Runtime};
use crate::sync_primitives::{EventPayload, EventQueueSend};

/// Largest listener or service data buffer accepted; longer buffers
/// are refused with `CELL_EINVAL` and a named diagnostic.
pub(crate) const SYS_CONFIG_DATA_CAP: u64 = 0x1000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ConfigHandle {
    pub queue_id: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ConfigService {
    pub service_id: u64,
    pub user_id: u64,
    pub verbosity: u64,
    pub data: Vec<u8>,
    pub registered: bool,
    /// Registration order; replays to a new listener follow it.
    pub order: u64,
}

impl ConfigService {
    /// Buffer a `sys_config_get_service_event` call must offer;
    /// reported in the queued event's `data3` whether or not the
    /// service is still registered. It overstates both record lengths:
    /// see [`Self::record_len`] and
    /// [`SYS_CONFIG_SERVICE_EVENT_UNREGISTERED_LEN`].
    fn announced_len(&self) -> usize {
        SYS_CONFIG_SERVICE_EVENT_ANNOUNCED_HEAD_LEN + self.data.len()
    }

    /// Bytes `sys_config_get_service_event` writes for a registered
    /// service; seven short of [`Self::announced_len`].
    fn record_len(&self) -> usize {
        SYS_CONFIG_SERVICE_EVENT_HEAD_LEN + self.data.len()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ConfigListener {
    pub handle: u32,
    /// Captured at add time: a closed handle's listeners keep
    /// delivering to the queue it was bound to.
    pub queue_id: u32,
    pub service_id: u64,
    pub min_verbosity: u64,
    pub listener_type: u32,
    pub data: Vec<u8>,
    /// Events created for this listener so far.
    pub delivered: u32,
}

impl ConfigListener {
    /// A once listener fires a single time, the id and verbosity must
    /// fit, and pad-manager events reach only listeners whose buffer
    /// leads with `0x01`.
    fn matches(&self, service: &ConfigService) -> bool {
        if self.listener_type == SYS_CONFIG_SERVICE_LISTENER_ONCE && self.delivered > 0 {
            return false;
        }
        if self.service_id != service.service_id || self.min_verbosity > service.verbosity {
            return false;
        }
        if self.service_id == SYS_CONFIG_SERVICE_PADMANAGER && self.data.first() != Some(&0x01) {
            return false;
        }
        true
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ConfigEvent {
    pub handle: u32,
    pub listener: u32,
    pub service: u32,
    /// Registration state the queued event's `data2` announced. The
    /// record read back later reports the service's live state
    /// instead.
    pub registered: bool,
}

/// Listener parameters decoded from `sys_config_add_service_listener`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::host) struct ListenerSpec {
    pub service_id: u64,
    pub min_verbosity: u64,
    pub in_ptr: u32,
    pub size: u64,
    pub listener_type: u32,
}

/// Service parameters decoded from `sys_config_register_service`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::host) struct ServiceSpec {
    pub service_id: u64,
    pub user_id: u64,
    pub verbosity: u64,
    pub data_ptr: u32,
    pub size: u64,
}

/// Handles, services, listeners, and the events a listener has been
/// sent and may still read back.
///
/// A service stays while it is registered or any event still refers
/// to it; an event stays until its listener is removed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct ConfigTable {
    handles: BTreeMap<u32, ConfigHandle>,
    services: BTreeMap<u32, ConfigService>,
    listeners: BTreeMap<u32, ConfigListener>,
    events: BTreeMap<u32, ConfigEvent>,
    next_event_id: u32,
    next_order: u64,
    /// The pad-manager services have been registered (first open).
    seeded: bool,
}

impl ConfigTable {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// True until the first `sys_config_open`; the state hash skips a
    /// pristine table.
    pub(crate) fn is_pristine(&self) -> bool {
        *self == Self::default()
    }

    #[cfg(test)]
    pub(crate) fn seeded(&self) -> bool {
        self.seeded
    }

    pub(crate) fn handle(&self, id: u32) -> Option<ConfigHandle> {
        self.handles.get(&id).copied()
    }

    pub(crate) fn service(&self, id: u32) -> Option<&ConfigService> {
        self.services.get(&id)
    }

    pub(crate) fn event(&self, id: u32) -> Option<ConfigEvent> {
        self.events.get(&id).copied()
    }

    #[cfg(test)]
    pub(crate) fn service_count(&self) -> usize {
        self.services.len()
    }

    #[cfg(test)]
    pub(crate) fn event_count(&self) -> usize {
        self.events.len()
    }

    fn insert_handle(&mut self, id: u32, queue_id: u32) {
        let prior = self.handles.insert(id, ConfigHandle { queue_id });
        debug_assert!(prior.is_none(), "config handle {id:#x} minted twice");
    }

    fn insert_service(&mut self, id: u32, spec: ServiceSpec, data: Vec<u8>) {
        let order = self.next_order;
        self.next_order += 1;
        let prior = self.services.insert(
            id,
            ConfigService {
                service_id: spec.service_id,
                user_id: spec.user_id,
                verbosity: spec.verbosity,
                data,
                registered: true,
                order,
            },
        );
        debug_assert!(prior.is_none(), "config service {id:#x} minted twice");
    }

    fn insert_listener(&mut self, id: u32, listener: ConfigListener) {
        let prior = self.listeners.insert(id, listener);
        debug_assert!(prior.is_none(), "config listener {id:#x} minted twice");
    }

    /// Registered services `listener` matches, oldest registration
    /// first.
    ///
    /// An unregistered service is never replayed, even while another
    /// listener still holds events for it. Unregistering withdraws the
    /// service before any notification goes out.
    fn matching_services(&self, listener: u32) -> Vec<u32> {
        let Some(l) = self.listeners.get(&listener) else {
            return vec![];
        };
        let mut hits: Vec<(u64, u32)> = self
            .services
            .iter()
            .filter(|(_, s)| s.registered && l.matches(s))
            .map(|(id, s)| (s.order, *id))
            .collect();
        hits.sort_unstable();
        hits.into_iter().map(|(_, id)| id).collect()
    }

    /// Listeners `service` matches, in listener-id order.
    fn matching_listeners(&self, service: u32) -> Vec<u32> {
        let Some(s) = self.services.get(&service) else {
            return vec![];
        };
        self.listeners
            .iter()
            .filter(|(_, l)| l.matches(s))
            .map(|(id, _)| *id)
            .collect()
    }

    /// Mint the event for `(listener, service)` and return the queue
    /// and payload the host must send. A once listener that already
    /// fired, or a match that no longer holds, returns `None`.
    fn stage_event(&mut self, listener: u32, service: u32) -> Option<(u32, u32, EventPayload)> {
        let (queue_id, handle) = {
            let l = self.listeners.get(&listener)?;
            let s = self.services.get(&service)?;
            if !l.matches(s) {
                return None;
            }
            (l.queue_id, l.handle)
        };
        let id = self.next_event_id;
        self.next_event_id = self.next_event_id.wrapping_add(1);
        let s = &self.services[&service];
        let registered = s.registered;
        let payload = EventPayload {
            source: SYS_CONFIG_EVENT_SOURCE_SERVICE,
            data1: u64::from(handle),
            data2: (u64::from(registered) << 32) | u64::from(id),
            data3: s.announced_len() as u64,
        };
        let prior = self.events.insert(
            id,
            ConfigEvent {
                handle,
                listener,
                service,
                registered,
            },
        );
        // The event counter wrapped onto an id a listener still holds.
        debug_assert!(prior.is_none(), "config event {id:#x} minted twice");
        let l = self
            .listeners
            .get_mut(&listener)
            .expect("listener present: looked up above");
        l.delivered += 1;
        Some((id, queue_id, payload))
    }

    /// Undo [`Self::stage_event`] after the queue refused the send.
    fn discard_event(&mut self, event: u32) {
        if let Some(ev) = self.events.remove(&event) {
            if let Some(l) = self.listeners.get_mut(&ev.listener) {
                l.delivered = l.delivered.saturating_sub(1);
            }
            self.collect_service(ev.service);
        }
    }

    /// Drop an unregistered service nothing refers to any more.
    fn collect_service(&mut self, service: u32) {
        let unreferenced = !self.events.values().any(|e| e.service == service);
        if unreferenced && self.services.get(&service).is_some_and(|s| !s.registered) {
            self.services.remove(&service);
        }
    }

    fn remove_handle(&mut self, id: u32) -> bool {
        self.handles.remove(&id).is_some()
    }

    /// Removing a listener drops every event it was sent.
    fn remove_listener(&mut self, id: u32) -> bool {
        if self.listeners.remove(&id).is_none() {
            return false;
        }
        let dropped: Vec<(u32, u32)> = self
            .events
            .iter()
            .filter(|(_, e)| e.listener == id)
            .map(|(ev, e)| (*ev, e.service))
            .collect();
        for (ev, service) in dropped {
            self.events.remove(&ev);
            self.collect_service(service);
        }
        true
    }

    /// Flip a service to unregistered; it is collected once no event
    /// refers to it. `false` for an unknown or already-unregistered
    /// service.
    fn unregister(&mut self, id: u32) -> bool {
        match self.services.get_mut(&id) {
            Some(s) if s.registered => {
                s.registered = false;
                true
            }
            _ => false,
        }
    }

    /// `sys_config_service_event_t` for `event`, `None` when either
    /// side of the event is gone.
    ///
    /// The `registered` field and the record's length follow the
    /// service's state at read time; see `ConfigEvent::registered`.
    fn record(&self, event: u32) -> Option<Vec<u8>> {
        let ev = self.events.get(&event)?;
        let s = self.services.get(&ev.service)?;
        let mut out = Vec::with_capacity(s.record_len());
        out.extend_from_slice(&ev.listener.to_be_bytes());
        out.extend_from_slice(&u32::from(s.registered).to_be_bytes());
        out.extend_from_slice(&s.service_id.to_be_bytes());
        out.extend_from_slice(&s.user_id.to_be_bytes());
        if s.registered {
            out.extend_from_slice(&s.verbosity.to_be_bytes());
            out.extend_from_slice(&(s.data.len() as u32).to_be_bytes());
            out.extend_from_slice(&0u32.to_be_bytes());
            out.extend_from_slice(&s.data);
            debug_assert_eq!(out.len(), s.record_len());
        } else {
            debug_assert_eq!(out.len(), SYS_CONFIG_SERVICE_EVENT_UNREGISTERED_LEN);
        }
        Some(out)
    }

    /// FNV-1a over every table, length-prefixed, plus the counters
    /// and the seed flag, via raw little-endian bytes per the host
    /// state-hash contract.
    pub(crate) fn state_hash(&self) -> u64 {
        let Self {
            handles,
            services,
            listeners,
            events,
            next_event_id,
            next_order,
            seeded,
        } = self;
        let mut hasher = cellgov_mem::Fnv1aHasher::new();
        hasher.write(&(handles.len() as u64).to_le_bytes());
        for (id, h) in handles {
            hasher.write(&id.to_le_bytes());
            hasher.write(&h.queue_id.to_le_bytes());
        }
        hasher.write(&(services.len() as u64).to_le_bytes());
        for (id, s) in services {
            hasher.write(&id.to_le_bytes());
            hasher.write(&s.service_id.to_le_bytes());
            hasher.write(&s.user_id.to_le_bytes());
            hasher.write(&s.verbosity.to_le_bytes());
            hasher.write(&[u8::from(s.registered)]);
            hasher.write(&s.order.to_le_bytes());
            hasher.write(&(s.data.len() as u64).to_le_bytes());
            hasher.write(&s.data);
        }
        hasher.write(&(listeners.len() as u64).to_le_bytes());
        for (id, l) in listeners {
            hasher.write(&id.to_le_bytes());
            hasher.write(&l.handle.to_le_bytes());
            hasher.write(&l.queue_id.to_le_bytes());
            hasher.write(&l.service_id.to_le_bytes());
            hasher.write(&l.min_verbosity.to_le_bytes());
            hasher.write(&l.listener_type.to_le_bytes());
            hasher.write(&l.delivered.to_le_bytes());
            hasher.write(&(l.data.len() as u64).to_le_bytes());
            hasher.write(&l.data);
        }
        hasher.write(&(events.len() as u64).to_le_bytes());
        for (id, e) in events {
            hasher.write(&id.to_le_bytes());
            hasher.write(&e.handle.to_le_bytes());
            hasher.write(&e.listener.to_le_bytes());
            hasher.write(&e.service.to_le_bytes());
            hasher.write(&[u8::from(e.registered)]);
        }
        hasher.write(&next_event_id.to_le_bytes());
        hasher.write(&next_order.to_le_bytes());
        hasher.write(&[u8::from(*seeded)]);
        hasher.finish()
    }
}

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
    pub(super) fn dispatch_config_open(
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
    pub(super) fn dispatch_config_close(&mut self, handle: u32) -> Lv2Dispatch {
        if self.state.config.remove_handle(handle) {
            Lv2Dispatch::immediate(0)
        } else {
            Lv2Dispatch::immediate(errno::CELL_ESRCH.into())
        }
    }

    /// `sys_config_get_service_event` (518).
    ///
    /// # Errors
    ///
    /// - `CELL_ESRCH` for an unknown handle, an unknown event, or an
    ///   event sent through a different handle.
    /// - `CELL_EAGAIN` when `size` is below the length the queued
    ///   event's `data3` announced, which exceeds the bytes written
    ///   (see [`SYS_CONFIG_SERVICE_EVENT_ANNOUNCED_HEAD_LEN`]).
    /// - `CELL_EFAULT` for a null destination.
    pub(super) fn dispatch_config_get_service_event(
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
            effects: vec![Effect::SharedWriteIntent {
                range: ByteRange::contiguous_u32(dst_ptr, record.len() as u32),
                bytes: WritePayload::from_slice(&record),
                ordering: PriorityClass::Normal,
                source: requester,
                source_time: tick,
            }],
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
    pub(super) fn dispatch_config_add_service_listener(
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
    pub(super) fn dispatch_config_remove_service_listener(
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
    pub(super) fn dispatch_config_register_service(
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
    pub(super) fn dispatch_config_unregister_service(
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
        let write = Effect::SharedWriteIntent {
            range: ByteRange::contiguous_u32(out_ptr, 4),
            bytes: WritePayload::from_slice(&id.to_be_bytes()),
            ordering: PriorityClass::Normal,
            source: requester,
            source_time: tick,
        };
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

#[cfg(test)]
#[path = "tests/config_tests.rs"]
mod tests;
