//! The config subscription store: handles, listeners, services and the events they queue.

use std::collections::BTreeMap;

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
    pub(super) handles: BTreeMap<u32, ConfigHandle>,
    pub(super) services: BTreeMap<u32, ConfigService>,
    pub(super) listeners: BTreeMap<u32, ConfigListener>,
    pub(super) events: BTreeMap<u32, ConfigEvent>,
    next_event_id: u32,
    next_order: u64,
    /// The pad-manager services have been registered (first open).
    pub(super) seeded: bool,
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
    pub(super) fn matching_listeners(&self, service: u32) -> Vec<u32> {
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
    pub(super) fn stage_event(
        &mut self,
        listener: u32,
        service: u32,
    ) -> Option<(u32, u32, EventPayload)> {
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
    pub(super) fn discard_event(&mut self, event: u32) {
        if let Some(ev) = self.events.remove(&event) {
            if let Some(l) = self.listeners.get_mut(&ev.listener) {
                l.delivered = l.delivered.saturating_sub(1);
            }
            self.collect_service(ev.service);
        }
    }

    /// Drop an unregistered service nothing refers to any more.
    pub(super) fn collect_service(&mut self, service: u32) {
        let unreferenced = !self.events.values().any(|e| e.service == service);
        if unreferenced && self.services.get(&service).is_some_and(|s| !s.registered) {
            self.services.remove(&service);
        }
    }

    pub(super) fn remove_handle(&mut self, id: u32) -> bool {
        self.handles.remove(&id).is_some()
    }

    /// Removing a listener drops every event it was sent.
    pub(super) fn remove_listener(&mut self, id: u32) -> bool {
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
    pub(super) fn unregister(&mut self, id: u32) -> bool {
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
    pub(super) fn record(&self, event: u32) -> Option<Vec<u8>> {
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
