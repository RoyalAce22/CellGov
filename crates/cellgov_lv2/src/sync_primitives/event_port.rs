//! Event ports: the send side of an LV2 event queue.
//!
//! A port is created unbound and later connected to exactly one queue,
//! locally by queue id (136) or across processes by ipc key (140).
//! Sends resolve through the binding, so an unconnected port cannot
//! deliver.

use cellgov_mem::lanes::{source, LaneMap, LaneValue, ObjectLanes};

/// One event port.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventPortEntry {
    port_type: u64,
    name: u64,
    queue: Option<u32>,
}

impl EventPortEntry {
    /// `SYS_EVENT_PORT_LOCAL` or `SYS_EVENT_PORT_IPC`.
    pub fn port_type(&self) -> u64 {
        self.port_type
    }

    /// Guest-supplied name; opaque to the model.
    pub fn name(&self) -> u64 {
        self.name
    }

    /// Bound queue id, `None` while unconnected.
    pub fn queue(&self) -> Option<u32> {
        self.queue
    }
}

/// Why a connect was refused. Variants map 1:1 onto the errno the
/// dispatch arm returns.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum EventPortConnectError {
    /// -> `CELL_ESRCH`.
    #[error("no event port with that id")]
    UnknownPort,
    /// -> `CELL_EINVAL`.
    #[error("port type does not permit this connect form")]
    WrongType,
    /// -> `CELL_EISCONN`.
    #[error("port is already connected to a queue")]
    AlreadyConnected,
}

/// Why a disconnect was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum EventPortDisconnectError {
    /// -> `CELL_ESRCH`.
    #[error("no event port with that id")]
    UnknownPort,
    /// -> `CELL_ENOTCONN`.
    #[error("port is not connected to a queue")]
    NotConnected,
}

/// Why a destroy was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum EventPortDestroyError {
    /// -> `CELL_ESRCH`.
    #[error("no event port with that id")]
    UnknownPort,
    /// -> `CELL_EISCONN`.
    #[error("port is still connected to a queue")]
    Connected,
}

/// Every live event port, keyed by kernel id.
#[derive(Debug, Clone)]
pub struct EventPortTable {
    ports: LaneMap<u32, EventPortEntry>,
}

impl Default for EventPortTable {
    fn default() -> Self {
        Self {
            ports: LaneMap::new(source::EVENT_PORT, u64::from),
        }
    }
}

impl LaneValue for EventPortEntry {
    fn lanes(&self, lanes: &mut ObjectLanes) {
        lanes.lane(1, 0, self.port_type);
        lanes.lane(2, 0, self.name);
        lanes.lane(3, 0, u64::from(self.queue.is_some()));
        lanes.lane(4, 0, self.queue.map_or(0, u64::from));
    }
}

impl EventPortTable {
    /// An empty table.
    pub fn new() -> Self {
        Self::default()
    }

    /// Caller owns id allocation via `Lv2Host::alloc_id`.
    pub fn create_with_id(&mut self, id: u32, port_type: u64, name: u64) {
        let prior = self.ports.insert(
            id,
            EventPortEntry {
                port_type,
                name,
                queue: None,
            },
        );
        debug_assert!(
            prior.is_none(),
            "event port id {id} already in table; alloc_id collision",
        );
    }

    /// `None` when no port carries `id`.
    pub fn lookup(&self, id: u32) -> Option<&EventPortEntry> {
        self.ports.get(id)
    }

    /// True when no ports are live.
    pub fn is_empty(&self) -> bool {
        self.ports.is_empty()
    }

    /// The table's partial of the sync-state sum.
    pub fn sync_partial(&self) -> u128 {
        self.ports.partial()
    }

    /// [`Self::sync_partial`] computed from every entry.
    pub fn sync_partial_from_scratch(&self) -> u128 {
        self.ports.partial_from_scratch()
    }

    /// Bind `id` to `queue_id`.
    ///
    /// `required_type` is the port type this connect form accepts;
    /// a port of any other type is refused, matching the oracle's
    /// per-form type gate.
    ///
    /// # Errors
    ///
    /// [`EventPortConnectError::AlreadyConnected`] leaves the existing
    /// binding intact -- a second connect never silently retargets a
    /// port that is already delivering somewhere.
    pub fn connect(
        &mut self,
        id: u32,
        queue_id: u32,
        required_type: u64,
    ) -> Result<(), EventPortConnectError> {
        let mut port = self
            .ports
            .get_mut(id)
            .ok_or(EventPortConnectError::UnknownPort)?;
        if port.port_type != required_type {
            return Err(EventPortConnectError::WrongType);
        }
        if port.queue.is_some() {
            return Err(EventPortConnectError::AlreadyConnected);
        }
        port.queue = Some(queue_id);
        Ok(())
    }

    /// # Errors
    ///
    /// [`EventPortDisconnectError::NotConnected`] when the port has no
    /// binding to drop.
    pub fn disconnect(&mut self, id: u32) -> Result<(), EventPortDisconnectError> {
        let mut port = self
            .ports
            .get_mut(id)
            .ok_or(EventPortDisconnectError::UnknownPort)?;
        if port.queue.take().is_none() {
            return Err(EventPortDisconnectError::NotConnected);
        }
        Ok(())
    }

    /// # Errors
    ///
    /// [`EventPortDestroyError::Connected`] keeps the port alive; the
    /// guest must disconnect first.
    pub fn destroy(&mut self, id: u32) -> Result<(), EventPortDestroyError> {
        match self.ports.get(id) {
            None => Err(EventPortDestroyError::UnknownPort),
            Some(port) if port.queue.is_some() => Err(EventPortDestroyError::Connected),
            Some(_) => {
                self.ports.remove(id);
                Ok(())
            }
        }
    }

    /// Drop every binding that targets `queue_id`.
    ///
    /// # Cross-module contract
    ///
    /// The queue-destroy arm must call this, or a surviving port keeps
    /// a binding to a dead queue and its next send resolves to an id
    /// the queue table no longer knows.
    ///
    /// O(n) over the port table.
    pub fn unbind_queue(&mut self, queue_id: u32) {
        self.ports.for_each_mut(|_, port| {
            if port.queue == Some(queue_id) {
                port.queue = None;
            }
        });
    }
}

#[cfg(test)]
#[path = "tests/event_port_tests.rs"]
mod tests;
