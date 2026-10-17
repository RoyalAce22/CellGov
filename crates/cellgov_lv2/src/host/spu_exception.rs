//! The kernel's answer to an MFC exception on an SPU thread: the
//! group's exception event.
//!
//! In the `spu_list_bad_size` and `spu_list_bad_ea` console captures, a
//! thread's list put raises an MFC DMA alignment interrupt in a group
//! that connects its exception event to a PPU event queue. The kernel
//! stops the group's SPUs and sends the queue one event:
//!
//! - source [`event::GROUP_EXCEPTION_KEY`];
//! - data1: the group id in the high word, the thread id in the low;
//! - data2: the thread's SPU_NPC in the high word, the cause in the low;
//! - data3: MFC_DAR for an address fault, 0 for an alignment fault.
//!
//! The group stays running: a terminate answers OK and the join reads
//! the terminate. The `spu_ea_bound` capture reads the data-segment
//! cause and its DAR the same way.

use cellgov_event::UnitId;
use cellgov_ps3_abi::lv2::spu::event;

use crate::dispatch::PendingResponse;
use crate::host::Lv2Host;
use crate::sync_primitives::{EventPayload, EventQueueSend};

/// The exception event a group's thread raised, as the queue took it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpuGroupException {
    /// The PPU unit the event woke, with the response it returns.
    pub wake: Option<(UnitId, PendingResponse)>,
}

impl Lv2Host {
    /// The group of SPU thread `unit` and the queue its exception event
    /// goes to, when the thread's group has one connected.
    pub fn spu_exception_queue(&self, unit: UnitId) -> Option<(u32, u32)> {
        let group = self.state.groups.live_group_of(unit)?;
        let queue = self
            .state
            .groups
            .group_event_queue(group, event::GROUP_EXCEPTION)?;
        Some((group, queue))
    }

    /// Send the exception event SPU thread `unit` raised with `cause` to
    /// its group's exception queue. `npc` is the thread's SPU_NPC once
    /// stopped, and `dar` the faulting effective address, 0 for an
    /// alignment fault.
    ///
    /// `None` when the thread's group has no exception queue. A full or
    /// vanished queue drops the event under a named invariant break: no
    /// capture shows what the kernel does then.
    pub fn raise_spu_group_exception(
        &mut self,
        unit: UnitId,
        npc: u32,
        cause: u32,
        dar: u64,
    ) -> Option<SpuGroupException> {
        let (group, queue) = self.spu_exception_queue(unit)?;
        let thread = self.state.groups.live_thread_of(unit).unwrap_or_else(|| {
            self.log_invariant_break(
                "host.spu_exception_thread_unknown",
                format_args!("SPU unit {unit:?} of group 0x{group:08x} has no thread id"),
            );
            0
        });
        let payload = EventPayload {
            source: event::GROUP_EXCEPTION_KEY,
            data1: u64::from(group) << 32 | u64::from(thread),
            data2: u64::from(npc) << 32 | u64::from(cause),
            data3: dar,
        };
        let wake = match self
            .state
            .event_queues
            .send_and_wake_or_enqueue(queue, payload)
        {
            EventQueueSend::Enqueued => None,
            EventQueueSend::Woke {
                new_owner,
                out_ptr,
                payload,
            } => self
                .resolve_wake_thread(new_owner, "spu_group_exception.Woke")
                .map(|woken| {
                    (
                        woken,
                        PendingResponse::EventQueueReceive {
                            out_ptr,
                            payload: Some(payload),
                        },
                    )
                }),
            EventQueueSend::Unknown | EventQueueSend::Full => {
                self.log_invariant_break(
                    "host.spu_exception_event_dropped",
                    format_args!(
                        "group 0x{group:08x}: exception queue 0x{queue:08x} took no event"
                    ),
                );
                None
            }
        };
        Some(SpuGroupException { wake })
    }
}

#[cfg(test)]
#[path = "tests/spu_exception_tests.rs"]
mod tests;
