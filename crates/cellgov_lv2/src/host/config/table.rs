//! The config subscription store: handles, listeners, services and the events they queue.

use cellgov_mem::lanes::{self, source, LaneMap, LaneValue, ObjectLanes};

use cellgov_ps3_abi::lv2::config::{
    SYS_CONFIG_EVENT_SOURCE_SERVICE, SYS_CONFIG_SERVICE_EVENT_ANNOUNCED_HEAD_LEN,
    SYS_CONFIG_SERVICE_EVENT_HEAD_LEN, SYS_CONFIG_SERVICE_EVENT_UNREGISTERED_LEN,
    SYS_CONFIG_SERVICE_LISTENER_ONCE, SYS_CONFIG_SERVICE_PADMANAGER,
};

use crate::sync_primitives::EventPayload;

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
    pub(super) fn announced_len(&self) -> usize {
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
    pub(super) fn matches(&self, service: &ConfigService) -> bool {
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

impl LaneValue for ConfigHandle {
    fn lanes(&self, lanes: &mut ObjectLanes) {
        lanes.lane(1, 0, u64::from(self.queue_id));
    }
}

impl LaneValue for ConfigService {
    fn lanes(&self, lanes: &mut ObjectLanes) {
        lanes.lane(1, 0, self.service_id);
        lanes.lane(2, 0, self.user_id);
        lanes.lane(3, 0, self.verbosity);
        lanes.lane(4, 0, u64::from(self.registered));
        lanes.lane(5, 0, self.order);
        lanes.bytes(6, &[], &self.data);
    }
}

impl LaneValue for ConfigListener {
    fn lanes(&self, lanes: &mut ObjectLanes) {
        lanes.lane(1, 0, u64::from(self.handle));
        lanes.lane(2, 0, u64::from(self.queue_id));
        lanes.lane(3, 0, self.service_id);
        lanes.lane(4, 0, self.min_verbosity);
        lanes.lane(5, 0, u64::from(self.listener_type));
        lanes.lane(6, 0, u64::from(self.delivered));
        lanes.bytes(7, &[], &self.data);
    }
}

impl LaneValue for ConfigEvent {
    fn lanes(&self, lanes: &mut ObjectLanes) {
        lanes.lane(1, 0, u64::from(self.handle));
        lanes.lane(2, 0, u64::from(self.listener));
        lanes.lane(3, 0, u64::from(self.service));
        lanes.lane(4, 0, u64::from(self.registered));
    }
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
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ConfigTable {
    pub(super) handles: LaneMap<u32, ConfigHandle>,
    pub(super) services: LaneMap<u32, ConfigService>,
    pub(super) listeners: LaneMap<u32, ConfigListener>,
    pub(super) events: LaneMap<u32, ConfigEvent>,
    next_event_id: u32,
    next_order: u64,
    /// The pad-manager services have been registered (first open).
    pub(super) seeded: bool,
}

impl Default for ConfigTable {
    fn default() -> Self {
        Self {
            handles: LaneMap::new(source::CONFIG_HANDLE, u64::from),
            services: LaneMap::new(source::CONFIG_SERVICE, u64::from),
            listeners: LaneMap::new(source::CONFIG_LISTENER, u64::from),
            events: LaneMap::new(source::CONFIG_EVENT, u64::from),
            next_event_id: 0,
            next_order: 0,
            seeded: false,
        }
    }
}

impl ConfigTable {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// True until the first `sys_config_open`.
    #[cfg(test)]
    pub(crate) fn is_pristine(&self) -> bool {
        *self == Self::default()
    }

    #[cfg(test)]
    pub(crate) fn seeded(&self) -> bool {
        self.seeded
    }

    pub(crate) fn handle(&self, id: u32) -> Option<ConfigHandle> {
        self.handles.get(id).copied()
    }

    pub(crate) fn service(&self, id: u32) -> Option<&ConfigService> {
        self.services.get(id)
    }

    pub(crate) fn event(&self, id: u32) -> Option<ConfigEvent> {
        self.events.get(id).copied()
    }

    #[cfg(test)]
    pub(crate) fn service_count(&self) -> usize {
        self.services.len()
    }

    #[cfg(test)]
    pub(crate) fn event_count(&self) -> usize {
        self.events.len()
    }

    pub(super) fn insert_handle(&mut self, id: u32, queue_id: u32) {
        let prior = self.handles.insert(id, ConfigHandle { queue_id });
        debug_assert!(prior.is_none(), "config handle {id:#x} minted twice");
    }

    pub(super) fn insert_service(&mut self, id: u32, spec: ServiceSpec, data: Vec<u8>) {
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

    pub(super) fn insert_listener(&mut self, id: u32, listener: ConfigListener) {
        let prior = self.listeners.insert(id, listener);
        debug_assert!(prior.is_none(), "config listener {id:#x} minted twice");
    }

    /// Registered services `listener` matches, oldest registration
    /// first.
    ///
    /// An unregistered service is never replayed, even while another
    /// listener still holds events for it. Unregistering withdraws the
    /// service before any notification goes out.
    pub(super) fn matching_services(&self, listener: u32) -> Vec<u32> {
        let Some(l) = self.listeners.get(listener) else {
            return vec![];
        };
        let mut hits: Vec<(u64, u32)> = self
            .services
            .iter()
            .filter(|(_, s)| s.registered && l.matches(s))
            .map(|(id, s)| (s.order, id))
            .collect();
        hits.sort_unstable();
        hits.into_iter().map(|(_, id)| id).collect()
    }

    /// Listeners `service` matches, in listener-id order.
    pub(super) fn matching_listeners(&self, service: u32) -> Vec<u32> {
        let Some(s) = self.services.get(service) else {
            return vec![];
        };
        self.listeners
            .iter()
            .filter(|(_, l)| l.matches(s))
            .map(|(id, _)| id)
            .collect()
    }

    /// Mint the event for `(listener, service)` and return the queue
    /// and payload the host must send. A once listener that already
    /// fired, or a match that no longer holds, returns `None`.
    pub(super) fn stage_event(
        &mut self,
        listener: u32,
        service: u32,
    ) -> Option<(u32, u32, EventPayload)> {
        let (queue_id, handle, registered, announced_len) = {
            let l = self.listeners.get(listener)?;
            let s = self.services.get(service)?;
            if !l.matches(s) {
                return None;
            }
            (l.queue_id, l.handle, s.registered, s.announced_len())
        };
        let id = self.next_event_id;
        self.next_event_id = self.next_event_id.wrapping_add(1);
        let payload = EventPayload {
            source: SYS_CONFIG_EVENT_SOURCE_SERVICE,
            data1: u64::from(handle),
            data2: (u64::from(registered) << 32) | u64::from(id),
            data3: announced_len as u64,
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
        if let Some(mut l) = self.listeners.get_mut(listener) {
            l.delivered += 1;
        } else {
            // The lookup at the top found the listener, and nothing since removes one.
            debug_assert!(
                false,
                "config listener {listener:#x} vanished while staging"
            );
        }
        Some((id, queue_id, payload))
    }

    /// Undo [`Self::stage_event`] after the queue refused the send.
    pub(super) fn discard_event(&mut self, event: u32) {
        if let Some(ev) = self.events.remove(event) {
            if let Some(mut l) = self.listeners.get_mut(ev.listener) {
                l.delivered = l.delivered.saturating_sub(1);
            }
            self.collect_service(ev.service);
        }
    }

    /// Drop an unregistered service nothing refers to any more.
    pub(super) fn collect_service(&mut self, service: u32) {
        let unreferenced = !self.events.values().any(|e| e.service == service);
        if unreferenced && self.services.get(service).is_some_and(|s| !s.registered) {
            self.services.remove(service);
        }
    }

    pub(super) fn remove_handle(&mut self, id: u32) -> bool {
        self.handles.remove(id).is_some()
    }

    /// Removing a listener drops every event it was sent.
    pub(super) fn remove_listener(&mut self, id: u32) -> bool {
        if self.listeners.remove(id).is_none() {
            return false;
        }
        let dropped: Vec<(u32, u32)> = self
            .events
            .iter()
            .filter(|(_, e)| e.listener == id)
            .map(|(ev, e)| (ev, e.service))
            .collect();
        for (ev, service) in dropped {
            self.events.remove(ev);
            self.collect_service(service);
        }
        true
    }

    /// Flip a service to unregistered; it is collected once no event
    /// refers to it. `false` for an unknown or already-unregistered
    /// service.
    pub(super) fn unregister(&mut self, id: u32) -> bool {
        match self.services.get_mut(id) {
            Some(mut s) if s.registered => {
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
    pub(super) fn record(&self, event: u32) -> Option<Vec<u8>> {
        let ev = self.events.get(event)?;
        let s = self.services.get(ev.service)?;
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

    /// The table's partial of the sync-state sum: the handles, services,
    /// listeners and events, plus the two counters and the seed flag.
    pub(crate) fn sync_partial(&self) -> u128 {
        self.handles
            .partial()
            .wrapping_add(self.services.partial())
            .wrapping_add(self.listeners.partial())
            .wrapping_add(self.events.partial())
            .wrapping_add(self.counters_term())
    }

    /// [`Self::sync_partial`] computed from every entry.
    pub(crate) fn sync_partial_from_scratch(&self) -> u128 {
        self.handles
            .partial_from_scratch()
            .wrapping_add(self.services.partial_from_scratch())
            .wrapping_add(self.listeners.partial_from_scratch())
            .wrapping_add(self.events.partial_from_scratch())
            .wrapping_add(self.counters_term())
    }

    /// The counters and the seed flag, objects 0 to 2.
    fn counters_term(&self) -> u128 {
        lanes::value_term(source::CONFIG_COUNTERS, 0, &self.next_event_id)
            .wrapping_add(lanes::value_term(
                source::CONFIG_COUNTERS,
                1,
                &self.next_order,
            ))
            .wrapping_add(lanes::value_term(
                source::CONFIG_COUNTERS,
                2,
                &u64::from(self.seeded),
            ))
    }
}
