//! SPU event-binding LV2 dispatch: the event queues a thread group or
//! one of its threads connects to, and the queues an SPU thread binds
//! under numbers of its own.
//!
//! The arms keep the connections; nothing here delivers an event. The
//! SPU-side producer of a port event is a separate capability.

use cellgov_effects::{Effect, WritePayload};
use cellgov_event::UnitId;
use cellgov_mem::ByteRange;
use cellgov_ps3_abi::lv2::errno;
use cellgov_ps3_abi::lv2::spu::event;
use cellgov_time::GuestTicks;

use crate::dispatch::Lv2Dispatch;
use crate::host::Lv2Host;
use crate::thread_group::{BindQueueError, UnbindQueueError};

impl Lv2Host {
    /// `sys_spu_thread_group_connect_event`
    /// ([`cellgov_ps3_abi::lv2::syscall::SPU_THREAD_GROUP_CONNECT_EVENT`]):
    /// connect `queue_id` as the queue the group's `event_type` events
    /// reach.
    ///
    /// The group type is not modeled, so every group counts as a plain
    /// group. The arm refuses the system-module type with a named
    /// invariant break. The gate order is unestablished.
    ///
    /// # Errors
    ///
    /// - `CELL_ESRCH` when no group has `group_id`.
    /// - `CELL_EINVAL` when `event_type` is neither the run nor the
    ///   exception type.
    /// - `CELL_EBUSY` when the event type already has a queue.
    /// - `CELL_ESRCH` when no queue has `queue_id`. This check comes
    ///   after the busy check.
    pub(super) fn dispatch_group_connect_event(
        &mut self,
        group_id: u32,
        queue_id: u32,
        event_type: u32,
    ) -> Lv2Dispatch {
        if self.state.groups.get(group_id).is_none() {
            return Lv2Dispatch::immediate(errno::CELL_ESRCH.into());
        }
        match event_type {
            event::GROUP_RUN | event::GROUP_EXCEPTION => {}
            event::GROUP_SYSTEM_MODULE => {
                self.log_invariant_break(
                    "dispatch.spu_group_connect_event_system_module",
                    format_args!(
                        "sys_spu_thread_group_connect_event: group {group_id:#x} asked for the \
                         system-module event type, and the group type that admits it is not \
                         modelled; returning CELL_EINVAL"
                    ),
                );
                return Lv2Dispatch::immediate(errno::CELL_EINVAL.into());
            }
            _ => return Lv2Dispatch::immediate(errno::CELL_EINVAL.into()),
        }
        if self
            .state
            .groups
            .group_event_queue(group_id, event_type)
            .is_some()
        {
            return Lv2Dispatch::immediate(errno::CELL_EBUSY.into());
        }
        if self.state.event_queues.lookup(queue_id).is_none() {
            return Lv2Dispatch::immediate(errno::CELL_ESRCH.into());
        }
        self.state
            .groups
            .set_group_event_queue(group_id, event_type, Some(queue_id));
        Lv2Dispatch::immediate(0)
    }

    /// `sys_spu_thread_group_disconnect_event`
    /// ([`cellgov_ps3_abi::lv2::syscall::SPU_THREAD_GROUP_DISCONNECT_EVENT`]):
    /// drop the queue connected to the group's `event_type`.
    ///
    /// An event type the group never connected, or one outside the three
    /// group types, answers `CELL_OK`. The arm checks only the group.
    /// Whether the kernel checks more is unestablished.
    ///
    /// # Errors
    ///
    /// - `CELL_ESRCH` when no group has `group_id`.
    pub(super) fn dispatch_group_disconnect_event(
        &mut self,
        group_id: u32,
        event_type: u32,
    ) -> Lv2Dispatch {
        if !self
            .state
            .groups
            .set_group_event_queue(group_id, event_type, None)
        {
            return Lv2Dispatch::immediate(errno::CELL_ESRCH.into());
        }
        Lv2Dispatch::immediate(0)
    }

    /// `sys_spu_thread_connect_event`
    /// ([`cellgov_ps3_abi::lv2::syscall::SPU_THREAD_CONNECT_EVENT`]):
    /// connect `queue_id` to event port `port` of an initialized thread.
    ///
    /// # Errors
    ///
    /// - `CELL_ESRCH` when no initialized thread has `thread_id` or no
    ///   queue has `queue_id`.
    /// - `CELL_EINVAL` when `event_type` is not the user type or `port`
    ///   is not below [`event::PORT_COUNT`].
    /// - `CELL_EISCONN` when the port already has a queue.
    pub(super) fn dispatch_thread_connect_event(
        &mut self,
        thread_id: u32,
        queue_id: u32,
        event_type: u32,
        port: u32,
    ) -> Lv2Dispatch {
        if self.state.groups.thread_slot(thread_id).is_none()
            || self.state.event_queues.lookup(queue_id).is_none()
        {
            return Lv2Dispatch::immediate(errno::CELL_ESRCH.into());
        }
        if event_type != event::THREAD_USER || port >= event::PORT_COUNT {
            return Lv2Dispatch::immediate(errno::CELL_EINVAL.into());
        }
        if self
            .state
            .groups
            .thread_port_queue(thread_id, port)
            .is_some()
        {
            return Lv2Dispatch::immediate(errno::CELL_EISCONN.into());
        }
        self.state
            .groups
            .set_thread_port_queue(thread_id, port, Some(queue_id));
        Lv2Dispatch::immediate(0)
    }

    /// `sys_spu_thread_disconnect_event`
    /// ([`cellgov_ps3_abi::lv2::syscall::SPU_THREAD_DISCONNECT_EVENT`]):
    /// drop the queue connected to event port `port` of an initialized
    /// thread.
    ///
    /// # Errors
    ///
    /// - `CELL_ESRCH` when no initialized thread has `thread_id`.
    /// - `CELL_EINVAL` when `event_type` is not the user type or `port`
    ///   is not below [`event::PORT_COUNT`].
    /// - `CELL_ENOTCONN` when the port has no queue.
    pub(super) fn dispatch_thread_disconnect_event(
        &mut self,
        thread_id: u32,
        event_type: u32,
        port: u32,
    ) -> Lv2Dispatch {
        if self.state.groups.thread_slot(thread_id).is_none() {
            return Lv2Dispatch::immediate(errno::CELL_ESRCH.into());
        }
        if event_type != event::THREAD_USER || port >= event::PORT_COUNT {
            return Lv2Dispatch::immediate(errno::CELL_EINVAL.into());
        }
        if self
            .state
            .groups
            .thread_port_queue(thread_id, port)
            .is_none()
        {
            return Lv2Dispatch::immediate(errno::CELL_ENOTCONN.into());
        }
        self.state
            .groups
            .set_thread_port_queue(thread_id, port, None);
        Lv2Dispatch::immediate(0)
    }

    /// `sys_spu_thread_bind_queue`
    /// ([`cellgov_ps3_abi::lv2::syscall::SPU_THREAD_BIND_QUEUE`]): bind
    /// `queue_id` under `queue_number` on an initialized thread.
    ///
    /// The queue type is not modeled, so a queue created for PPU
    /// receivers binds like an SPU queue.
    ///
    /// # Errors
    ///
    /// - `CELL_ESRCH` when no initialized thread has `thread_id` or no
    ///   queue has `queue_id`.
    /// - `CELL_EBUSY` when the thread already binds `queue_number` or
    ///   already binds `queue_id` under another number.
    /// - `CELL_EAGAIN` when the thread holds
    ///   [`event::QUEUE_BINDING_COUNT`] bindings.
    pub(super) fn dispatch_thread_bind_queue(
        &mut self,
        thread_id: u32,
        queue_id: u32,
        queue_number: u32,
    ) -> Lv2Dispatch {
        if self.state.groups.thread_slot(thread_id).is_none()
            || self.state.event_queues.lookup(queue_id).is_none()
        {
            return Lv2Dispatch::immediate(errno::CELL_ESRCH.into());
        }
        match self
            .state
            .groups
            .bind_thread_queue(thread_id, queue_number, queue_id)
        {
            Ok(()) => Lv2Dispatch::immediate(0),
            Err(BindQueueError::UnknownThread) => Lv2Dispatch::immediate(errno::CELL_ESRCH.into()),
            Err(BindQueueError::Busy) => Lv2Dispatch::immediate(errno::CELL_EBUSY.into()),
            Err(BindQueueError::Full) => Lv2Dispatch::immediate(errno::CELL_EAGAIN.into()),
        }
    }

    /// `sys_spu_thread_unbind_queue`
    /// ([`cellgov_ps3_abi::lv2::syscall::SPU_THREAD_UNBIND_QUEUE`]): drop
    /// the queue an initialized thread binds under `queue_number`.
    ///
    /// # Errors
    ///
    /// - `CELL_ESRCH` when no initialized thread has `thread_id`, or the
    ///   thread binds no queue under `queue_number`.
    pub(super) fn dispatch_thread_unbind_queue(
        &mut self,
        thread_id: u32,
        queue_number: u32,
    ) -> Lv2Dispatch {
        match self
            .state
            .groups
            .unbind_thread_queue(thread_id, queue_number)
        {
            Ok(()) => Lv2Dispatch::immediate(0),
            Err(UnbindQueueError::UnknownThread | UnbindQueueError::NotBound) => {
                Lv2Dispatch::immediate(errno::CELL_ESRCH.into())
            }
        }
    }

    /// `sys_spu_thread_group_connect_event_all_threads`
    /// ([`cellgov_ps3_abi::lv2::syscall::SPU_THREAD_GROUP_CONNECT_EVENT_ALL_THREADS`]):
    /// connect `queue_id` to one event port of every thread of the group,
    /// and write that port number to `port_ptr` as one byte. The port is
    /// the lowest one `request_mask` requests that no thread connected.
    ///
    /// The null-pointer check runs before any port changes, so a refused
    /// call connects nothing. Where the kernel places that check is
    /// unestablished.
    ///
    /// # Errors
    ///
    /// - `CELL_EINVAL` when `request_mask` is zero. This check comes
    ///   first.
    /// - `CELL_ESRCH` when no group has `group_id` or no queue has
    ///   `queue_id`.
    /// - `CELL_ESTAT` when a declared slot of the group is not yet
    ///   initialized.
    /// - `CELL_EFAULT` when `port_ptr` is null.
    /// - `CELL_EISCONN` when every requested port is connected on some
    ///   thread.
    pub(super) fn dispatch_group_connect_event_all_threads(
        &mut self,
        group_id: u32,
        queue_id: u32,
        request_mask: u64,
        port_ptr: u32,
        requester: UnitId,
        tick: GuestTicks,
    ) -> Lv2Dispatch {
        if request_mask == 0 {
            return Lv2Dispatch::immediate(errno::CELL_EINVAL.into());
        }
        let Some(group) = self.state.groups.get(group_id) else {
            return Lv2Dispatch::immediate(errno::CELL_ESRCH.into());
        };
        if self.state.event_queues.lookup(queue_id).is_none() {
            return Lv2Dispatch::immediate(errno::CELL_ESRCH.into());
        }
        if !group.is_initialized() {
            return Lv2Dispatch::immediate(errno::CELL_ESTAT.into());
        }
        if let Some(refusal) = self.efault_if_null(&[port_ptr]) {
            return refusal;
        }
        let Some(port) = (0..event::PORT_COUNT)
            .find(|&port| request_mask & (1u64 << port) != 0 && group.port_is_free(port))
        else {
            return Lv2Dispatch::immediate(errno::CELL_EISCONN.into());
        };
        self.state
            .groups
            .set_group_port_queue(group_id, port, Some(queue_id));
        Lv2Dispatch::Immediate {
            code: 0,
            effects: vec![Effect::shared_write(
                ByteRange::contiguous_u32(port_ptr, 1),
                WritePayload::from_slice(&[port as u8]),
                requester,
                tick,
            )],
        }
    }

    /// `sys_spu_thread_group_disconnect_event_all_threads`
    /// ([`cellgov_ps3_abi::lv2::syscall::SPU_THREAD_GROUP_DISCONNECT_EVENT_ALL_THREADS`]):
    /// drop the queue connected to event port `port` on every thread of
    /// the group. A port no thread connected answers `CELL_OK`.
    ///
    /// # Errors
    ///
    /// - `CELL_EINVAL` when `port` is not below [`event::PORT_COUNT`].
    ///   This check comes first.
    /// - `CELL_ESRCH` when no group has `group_id`.
    pub(super) fn dispatch_group_disconnect_event_all_threads(
        &mut self,
        group_id: u32,
        port: u32,
    ) -> Lv2Dispatch {
        if port >= event::PORT_COUNT {
            return Lv2Dispatch::immediate(errno::CELL_EINVAL.into());
        }
        if !self.state.groups.set_group_port_queue(group_id, port, None) {
            return Lv2Dispatch::immediate(errno::CELL_ESRCH.into());
        }
        Lv2Dispatch::immediate(0)
    }
}

#[cfg(test)]
#[path = "tests/spu_event_tests.rs"]
mod tests;
