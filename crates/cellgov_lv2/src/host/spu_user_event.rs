//! LV2's answer to a write to an SPU thread's outbound interrupt
//! mailbox: a user event on the queue connected to one of the thread's
//! event ports.
//!
//! The `spu_intr_mbox_to_event_queue` console capture shows LV2 read
//! the word as [`user_event`] lays it out. The event carries:
//!
//! - source [`event::THREAD_USER_KEY`];
//! - data1: the thread id;
//! - data2: the port in the high word, data0 in the low;
//! - data3: the word the thread wrote to `SPU_WrOutMbox`.
//!
//! A send reads one answer from its inbound mailbox:
//!
//! - `CELL_OK`: the queue takes the event;
//! - `CELL_EBUSY`: the queue is full;
//! - `CELL_ENOTCONN`: the port has no queue.
//!
//! [`user_event::THROW`] says what a throw gets.

use cellgov_event::UnitId;
use cellgov_ps3_abi::lv2::errno;
use cellgov_ps3_abi::lv2::spu::{event, user_event};

use crate::dispatch::PendingResponse;
use crate::host::Lv2Host;
use crate::spu_stop::SpuThreadError;
use crate::sync_primitives::{EventPayload, EventQueueSend};

/// The user event an SPU thread asked for, as LV2 served it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpuUserEvent {
    /// The word a send reads from `SPU_RdInMbox`. `None` for a throw.
    pub answer: Option<u32>,
    /// The PPU unit the event woke, with the response it returns.
    pub wake: Option<(UnitId, PendingResponse)>,
}

impl Lv2Host {
    /// Serves the word SPU thread `unit` wrote to `SPU_WrOutIntrMbox`.
    ///
    /// `data3` is the word the thread wrote to `SPU_WrOutMbox` before
    /// it, or `None` when that mailbox is empty.
    ///
    /// A send to a destroyed queue gets `CELL_ENOTCONN`, as if the port
    /// had no queue. The `spu_intr_mbox_edges` console capture destroys
    /// a connected queue and reads that answer.
    ///
    /// # Errors
    ///
    /// - [`SpuThreadError::UnservedInterruptMailbox`] when the word sets
    ///   [`user_event::OTHER_SERVICE`]. This check comes first.
    /// - [`SpuThreadError::InterruptMailboxWithoutData`] when `data3` is
    ///   `None`.
    pub fn send_spu_thread_user_event(
        &mut self,
        unit: UnitId,
        word: u32,
        data3: Option<u32>,
    ) -> Result<SpuUserEvent, SpuThreadError> {
        if word & user_event::OTHER_SERVICE != 0 {
            return Err(SpuThreadError::UnservedInterruptMailbox(word));
        }
        let data3 = data3.ok_or(SpuThreadError::InterruptMailboxWithoutData(word))?;
        let throw = word & user_event::THROW != 0;
        let port = word >> user_event::PORT_SHIFT & user_event::PORT_MASK;
        let thread = self.state.groups.live_thread_of(unit).unwrap_or_else(|| {
            self.log_invariant_break(
                "host.spu_user_event_thread_unknown",
                format_args!("SPU unit {unit:?} wrote its interrupt mailbox with no thread id"),
            );
            0
        });
        let answered = |code: errno::Lv2ErrCode| (!throw).then_some(code.code);
        let Some(queue) = self.state.groups.thread_port_queue(thread, port) else {
            return Ok(SpuUserEvent {
                answer: answered(errno::CELL_ENOTCONN),
                wake: None,
            });
        };
        let payload = EventPayload {
            source: event::THREAD_USER_KEY,
            data1: u64::from(thread),
            data2: u64::from(port) << 32 | u64::from(word & user_event::DATA0_MASK),
            data3: u64::from(data3),
        };
        let (code, wake) = match self
            .state
            .event_queues
            .send_and_wake_or_enqueue(queue, payload)
        {
            EventQueueSend::Enqueued => (errno::CELL_OK, None),
            EventQueueSend::Woke {
                new_owner,
                out_ptr,
                payload,
            } => (
                errno::CELL_OK,
                self.resolve_wake_thread(new_owner, "spu_user_event.Woke")
                    .map(|woken| {
                        (
                            woken,
                            PendingResponse::EventQueueReceive {
                                out_ptr,
                                payload: Some(payload),
                            },
                        )
                    }),
            ),
            EventQueueSend::Full => (errno::CELL_EBUSY, None),
            EventQueueSend::Unknown => (errno::CELL_ENOTCONN, None),
        };
        Ok(SpuUserEvent {
            answer: answered(code),
            wake,
        })
    }
}

#[cfg(test)]
#[path = "tests/spu_user_event_tests.rs"]
mod tests;
