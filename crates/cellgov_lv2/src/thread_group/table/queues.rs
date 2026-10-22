//! Event-queue connections: group event types, thread event ports,
//! and the queues a thread binds by number.

use super::ThreadGroupTable;
use crate::thread_group::{BindQueueError, UnbindQueueError};
use cellgov_ps3_abi::lv2::spu::event;
use std::collections::BTreeMap;

impl ThreadGroupTable {
    /// The queue connected to group event type `event_type`.
    pub fn group_event_queue(&self, group_id: u32, event_type: u32) -> Option<u32> {
        self.groups
            .get(group_id)?
            .event_queues
            .get(&event_type)
            .copied()
    }

    /// Connect group event type `event_type` to `queue`, or disconnect it
    /// with `None`. Returns `false` when no group has the id.
    pub fn set_group_event_queue(
        &mut self,
        group_id: u32,
        event_type: u32,
        queue: Option<u32>,
    ) -> bool {
        let Some(mut group) = self.groups.get_mut(group_id) else {
            return false;
        };
        match queue {
            Some(queue) => group.event_queues.insert(event_type, queue),
            None => group.event_queues.remove(&event_type),
        };
        true
    }

    /// The queue connected to event port `port` of an initialized thread.
    pub fn thread_port_queue(&self, thread_id: u32, port: u32) -> Option<u32> {
        let (group_id, slot) = self.slot_of_thread(thread_id)?;
        self.groups
            .get(group_id)?
            .slots
            .get(&slot)?
            .event_ports
            .get(&port)
            .copied()
    }

    /// Connect event port `port` of an initialized thread to `queue`, or
    /// disconnect it with `None`. Returns `false` when no initialized
    /// thread has the id.
    pub fn set_thread_port_queue(&mut self, thread_id: u32, port: u32, queue: Option<u32>) -> bool {
        let Some((group_id, slot)) = self.slot_of_thread(thread_id) else {
            return false;
        };
        let Some(mut group) = self.groups.get_mut(group_id) else {
            return false;
        };
        let Some(slot) = group.slots.get_mut(&slot) else {
            return false;
        };
        match queue {
            Some(queue) => slot.event_ports.insert(port, queue),
            None => slot.event_ports.remove(&port),
        };
        true
    }

    /// Connect event port `port` of every initialized thread of the group
    /// to `queue`, or disconnect it with `None`. Returns `false` when no
    /// group has the id.
    pub fn set_group_port_queue(&mut self, group_id: u32, port: u32, queue: Option<u32>) -> bool {
        let Some(mut group) = self.groups.get_mut(group_id) else {
            return false;
        };
        for slot in group.slots.values_mut() {
            match queue {
                Some(queue) => slot.event_ports.insert(port, queue),
                None => slot.event_ports.remove(&port),
            };
        }
        true
    }

    /// The queues an initialized thread names by number, by that number.
    pub fn thread_queue_bindings(&self, thread_id: u32) -> Option<&BTreeMap<u32, u32>> {
        let (group_id, slot) = self.slot_of_thread(thread_id)?;
        Some(&self.groups.get(group_id)?.slots.get(&slot)?.queue_bindings)
    }

    /// Bind `queue` to `number` on an initialized thread.
    ///
    /// # Errors
    ///
    /// See [`BindQueueError`]; an error binds nothing.
    pub fn bind_thread_queue(
        &mut self,
        thread_id: u32,
        number: u32,
        queue: u32,
    ) -> Result<(), BindQueueError> {
        let (group_id, slot) = self
            .slot_of_thread(thread_id)
            .ok_or(BindQueueError::UnknownThread)?;
        let mut group = self
            .groups
            .get_mut(group_id)
            .ok_or(BindQueueError::UnknownThread)?;
        let slot = group
            .slots
            .get_mut(&slot)
            .ok_or(BindQueueError::UnknownThread)?;
        if slot.queue_bindings.contains_key(&number)
            || slot.queue_bindings.values().any(|&bound| bound == queue)
        {
            return Err(BindQueueError::Busy);
        }
        if slot.queue_bindings.len() >= event::QUEUE_BINDING_COUNT {
            return Err(BindQueueError::Full);
        }
        slot.queue_bindings.insert(number, queue);
        Ok(())
    }

    /// Drop the binding an initialized thread holds under `number`.
    ///
    /// # Errors
    ///
    /// See [`UnbindQueueError`].
    pub fn unbind_thread_queue(
        &mut self,
        thread_id: u32,
        number: u32,
    ) -> Result<(), UnbindQueueError> {
        let (group_id, slot) = self
            .slot_of_thread(thread_id)
            .ok_or(UnbindQueueError::UnknownThread)?;
        let mut group = self
            .groups
            .get_mut(group_id)
            .ok_or(UnbindQueueError::UnknownThread)?;
        let slot = group
            .slots
            .get_mut(&slot)
            .ok_or(UnbindQueueError::UnknownThread)?;
        slot.queue_bindings
            .remove(&number)
            .map(|_| ())
            .ok_or(UnbindQueueError::NotBound)
    }

    /// Drop every group event connection, thread port connection and
    /// queue binding that targets `queue_id`.
    ///
    /// # Cross-module contract
    ///
    /// The queue-destroy arm must call this, or a surviving connection
    /// names a queue the queue table no longer knows.
    ///
    /// O(n) over every slot of every group.
    pub fn unbind_event_queue(&mut self, queue_id: u32) {
        self.groups.for_each_mut(|_, group| {
            group.event_queues.retain(|_, &mut queue| queue != queue_id);
            for slot in group.slots.values_mut() {
                slot.event_ports.retain(|_, &mut queue| queue != queue_id);
                slot.queue_bindings
                    .retain(|_, &mut queue| queue != queue_id);
            }
        });
    }
}
